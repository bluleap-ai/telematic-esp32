extern crate alloc;
use crate::alloc::string::ToString;
use crate::mender_mcu_client::addon::mender_addon::{MenderAddon, MenderAddonInstance};
use crate::mender_mcu_client::core::mender_api;
use crate::mender_mcu_client::core::mender_api::{mender_api_init, MenderApiConfig};
use crate::mender_mcu_client::core::mender_utils::{
    DeploymentStatus, KeyStore, MenderResult, MenderStatus,
};
use crate::mender_mcu_client::platform::scheduler::mender_scheduler::{
    mender_scheduler_work_activate, mender_scheduler_work_create, mender_scheduler_work_deactivate,
    mender_scheduler_work_delete, MenderFuture, MenderSchedulerWorkContext,
};
use alloc::boxed::Box;
use alloc::string::String;
use core::pin::Pin;
use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use esp_hal::rng::Trng;
use heapless::String as HString;
use serde::{Deserialize, Serialize};
use serde_json_core::de::from_str;

use crate::mender_mcu_client::mender_common::{
    MenderArtifactCallback, MenderCallback, MenderCallbackInfo,
};
use crate::mender_mcu_client::platform::flash::mender_flash;
use crate::mender_mcu_client::platform::scheduler::mender_scheduler;
use crate::mender_mcu_client::platform::storage::mender_storage;
use crate::mender_mcu_client::platform::tls::mender_tls;

use crate::cfg::mender_cfg::{
    CONFIG_MENDER_AUTH_POLL_INTERVAL, CONFIG_MENDER_UPDATE_POLL_INTERVAL,
};
use crate::mender_mcu_client::mender_common::{serde_bytes_str, serde_bytes_str_vec};
#[allow(unused_imports)]
use crate::{log_debug, log_error, log_info, log_warn};
use alloc::vec::Vec;
use core::future::Future;
use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct MenderClientConfig {
    pub identity: KeyStore,
    pub artifact_name: String,
    pub device_type: String,
    pub host: String,
    pub tenant_token: Option<String>,
    pub authentication_poll_interval: u32,
    pub update_poll_interval: u32,
    pub recommissioning: bool,
    pub device_update_done_reset: bool,
}

impl MenderClientConfig {
    pub fn new(
        identity: KeyStore,
        artifact_name: &str,
        device_type: &str,
        host: &str,
        tenant_token: Option<&str>,
    ) -> Self {
        Self {
            identity,
            artifact_name: artifact_name.to_string(),
            device_type: device_type.to_string(),
            host: host.to_string(),
            tenant_token: tenant_token.map(|s| s.to_string()),
            authentication_poll_interval: 0,
            update_poll_interval: 0,
            recommissioning: false,
            device_update_done_reset: false,
        }
    }

    pub fn with_host(mut self, host: &str) -> Self {
        self.host = host.to_string();
        self
    }

    pub fn with_auth_interval(mut self, interval: u32) -> Self {
        self.authentication_poll_interval = interval;
        self
    }

    pub fn with_update_interval(mut self, interval: u32) -> Self {
        self.update_poll_interval = interval;
        self
    }

    pub fn with_recommissioning(mut self, recommissioning: bool) -> Self {
        self.recommissioning = recommissioning;
        self
    }

    pub fn with_device_update_done_reset(mut self, device_update_done_reset: bool) -> Self {
        self.device_update_done_reset = device_update_done_reset;
        self
    }
}

#[derive(Debug, Clone)]
pub struct MenderClientCallbacks {
    pub network_connect: fn() -> MenderResult<()>,
    pub network_release: fn() -> MenderResult<()>,
    pub authentication_success: fn() -> MenderResult<()>,
    pub authentication_failure: fn() -> MenderResult<()>,
    pub deployment_status: fn(status: DeploymentStatus, message: Option<&str>) -> MenderResult<()>,
    pub restart: fn() -> MenderResult<()>,
}

impl MenderClientCallbacks {
    pub fn new(
        network_connect: fn() -> MenderResult<()>,
        network_release: fn() -> MenderResult<()>,
        authentication_success: fn() -> MenderResult<()>,
        authentication_failure: fn() -> MenderResult<()>,
        deployment_status: fn(status: DeploymentStatus, message: Option<&str>) -> MenderResult<()>,
        restart: fn() -> MenderResult<()>,
    ) -> Self {
        Self {
            network_connect,
            network_release,
            authentication_success,
            authentication_failure,
            deployment_status,
            restart,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenderClientState {
    Initialization, // Perform initialization
    Authentication, // Perform authentication with the server
    Authenticated,  // Perform updates
}

static MENDER_CLIENT_NETWORK_COUNT: Mutex<CriticalSectionRawMutex, u8> = Mutex::new(0);

static MENDER_CLIENT_CONFIG: Mutex<CriticalSectionRawMutex, Option<MenderClientConfig>> =
    Mutex::new(None);

static MENDER_CLIENT_CALLBACKS: Mutex<CriticalSectionRawMutex, Option<MenderClientCallbacks>> =
    Mutex::new(None);

// Static client state
static MENDER_CLIENT_STATE: Mutex<CriticalSectionRawMutex, MenderClientState> =
    Mutex::new(MenderClientState::Initialization);

// Add this with other static variables at the top
static MENDER_CLIENT_WORK: Mutex<CriticalSectionRawMutex, Option<MenderSchedulerWorkContext>> =
    Mutex::new(None);

// Static storage for addons - using () for both generic parameters
static MENDER_CLIENT_ADDONS: Mutex<CriticalSectionRawMutex, Vec<&'static dyn MenderAddon>> =
    Mutex::new(Vec::new());

// Define JSON-compatible data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeploymentData {
    #[serde(with = "serde_bytes_str")]
    id: String, // "026ffb94-e30a-4f1e-9155-71cb1a093532"
    #[serde(with = "serde_bytes_str")]
    artifact_name: String, // "mender-artifact-026ffb94-e30a-4f1e-9155-71cb1a093532"
    #[serde(with = "serde_bytes_str_vec")]
    types: Vec<String>, // ["rootfs-image"]
}

// Static storage
static MENDER_CLIENT_DEPLOYMENT_DATA: Mutex<CriticalSectionRawMutex, Option<DeploymentData>> =
    Mutex::new(None);

pub struct StaticTrng(&'static mut Trng<'static>);
impl StaticTrng {
    pub fn get_trng(&mut self) -> &mut Trng<'static> {
        self.0
    }
}

unsafe impl Send for StaticTrng {}
unsafe impl Sync for StaticTrng {}
// Add this with other static variables at the top
pub static MENDER_CLIENT_RNG: Mutex<CriticalSectionRawMutex, Option<StaticTrng>> = Mutex::new(None);

static MENDER_CLIENT_DEPLOYMENT_NEEDS_SET_PENDING_IMAGE: AtomicBool = AtomicBool::new(false);

static MENDER_CLIENT_DEPLOYMENT_NEEDS_RESTART: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct ArtifactTypeHandler {
    type_name: String,
    callback: &'static dyn MenderArtifactCallback,
    needs_restart: bool,
    artifact_name: String,
}

static MENDER_CLIENT_ARTIFACT_TYPES: Mutex<
    CriticalSectionRawMutex,
    Option<Vec<ArtifactTypeHandler>>,
> = Mutex::new(None);

// Make FlashCallback static
static FLASH_CALLBACK: FlashCallback = FlashCallback;

pub struct FlashCallback;

impl MenderArtifactCallback for FlashCallback {
    fn call<'a>(
        &'a self,
        // id: &'a str,
        // artifact_name: &'a str,
        // type_name: &'a str,
        // meta_data: &'a str,
        filename: &'a str,
        size: u32,
        data: &'a [u8],
        index: u32,
        length: u32,
        chksum: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = MenderResult<()>> + Send + 'a>> {
        Box::pin(async move {
            mender_client_download_artifact_flash_callback(
                // id,
                // artifact_name,
                // type_name,
                // meta_data,
                filename, size, data, index, length, chksum,
            )
            .await
        })
    }
}

pub async fn mender_client_init(
    config: &MenderClientConfig,
    callbacks: &MenderClientCallbacks,
    trng: &'static mut Trng<'static>,
    //stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>
    stack: Stack<'static>,
) -> MenderResult<()> {
    // Store RNG
    let mut rng_lock = MENDER_CLIENT_RNG.lock().await;
    *rng_lock = Some(StaticTrng(trng));

    // Validate configuration
    if config.artifact_name.is_empty()
        || config.device_type.is_empty()
        || config.identity.is_empty()
    {
        log_error!("Invalid artifact name, can't be empty");
        return Err(MenderStatus::Other);
    }

    // Copy configuration
    let mut saved_config = config.clone();

    // Print out identity contents
    log_info!("Identity contents: {:?}", saved_config.identity);

    // Handle host configuration
    saved_config.host = config.host.clone();

    // Validate host configuration
    if saved_config.host.is_empty() {
        log_error!("Invalid server host configuration, can't be empty");
        return Err(MenderStatus::Other);
    }

    if saved_config.host.ends_with('/') {
        log_error!("Invalid server host configuration, trailing '/' is not allowed");
        return Err(MenderStatus::Other);
    }

    saved_config.tenant_token = config.tenant_token.clone();

    // Set default poll intervals
    if config.authentication_poll_interval != 0 {
        saved_config.authentication_poll_interval = config.authentication_poll_interval;
    } else {
        saved_config.authentication_poll_interval = CONFIG_MENDER_AUTH_POLL_INTERVAL;
    }

    if config.update_poll_interval != 0 {
        saved_config.update_poll_interval = config.update_poll_interval;
    } else {
        saved_config.update_poll_interval = CONFIG_MENDER_UPDATE_POLL_INTERVAL;
    }

    let mender_api_config = MenderApiConfig {
        identity: saved_config.identity.clone(),
        artifact_name: saved_config.artifact_name.clone(),
        device_type: saved_config.device_type.clone(),
        host: saved_config.host.clone(),
        tenant_token: saved_config.tenant_token.as_ref().map(|s| s.to_string()),
    };

    if (mender_storage::mender_storage_init().await).is_err() {
        log_error!("Unable to initialize storage");
        return Err(MenderStatus::Other);
    }

    // Initialize TLS
    if (mender_tls::mender_tls_init()).is_err() {
        log_error!("Unable to initialize TLS");
        return Err(MenderStatus::Other);
    }

    mender_api_init(&mender_api_config, stack)
        .await
        .expect("Failed to init mender api");

    // Use the static FLASH_CALLBACK instead of creating a new instance
    if mender_client_register_artifact_type(
        "rootfs-image",
        &FLASH_CALLBACK,
        saved_config.device_update_done_reset,
        &saved_config.artifact_name,
    )
    .await
    .is_err()
    {
        log_error!("Unable to register 'rootfs-image' artifact type");
        return Err(MenderStatus::Other);
    }

    let work = mender_scheduler_work_create(
        mender_client_work,
        saved_config.authentication_poll_interval,
        "mender_client_update",
    )
    .expect("Failed to create work");

    let mut client_work = MENDER_CLIENT_WORK.lock().await;
    *client_work = Some(work);

    let mut conf = MENDER_CLIENT_CONFIG.lock().await;
    *conf = Some(saved_config);

    let mut cb = MENDER_CLIENT_CALLBACKS.lock().await;
    *cb = Some(callbacks.clone());

    Ok((MenderStatus::Ok, ()))
}

pub async fn mender_client_get_artifact_name() -> Option<String> {
    MENDER_CLIENT_CONFIG
        .lock()
        .await
        .as_ref()
        .map(|config| config.artifact_name.to_string())
}

pub async fn mender_client_get_device_type() -> Option<String> {
    MENDER_CLIENT_CONFIG
        .lock()
        .await
        .as_ref()
        .map(|config| config.device_type.to_string())
}

pub async fn mender_client_register_artifact_type(
    type_name: &str,
    callback: &'static dyn MenderArtifactCallback,
    needs_restart: bool,
    artifact_name: &str,
) -> MenderResult<()> {
    // Validate input
    if type_name.is_empty() {
        log_error!("Type name cannot be empty");
        return Err(MenderStatus::Failed);
    }

    // Create new artifact type handler
    let artifact_type = ArtifactTypeHandler {
        type_name: type_name.to_string(),
        callback,
        needs_restart,
        artifact_name: artifact_name.to_string(),
    };

    // Take mutex to protect access to the artifact types list
    let mut artifact_types = MENDER_CLIENT_ARTIFACT_TYPES.lock().await;

    // Initialize the vector if it doesn't exist
    if artifact_types.is_none() {
        *artifact_types = Some(Vec::new());
    }

    // Add the new artifact type to the list
    if let Some(types) = artifact_types.as_mut() {
        types.push(artifact_type);
    }

    Ok((MenderStatus::Ok, ()))
}

pub async fn mender_client_register_addon<C: 'static, CB: 'static>(
    addon: &'static MenderAddonInstance<C, CB>,
    config: Option<&'static C>,
    callbacks: Option<&'static CB>,
) -> MenderResult<()> {
    let mut addons = MENDER_CLIENT_ADDONS.lock().await;

    // Initialize the add-on
    (addon.init)(config, callbacks).await?;

    // Activate add-on if authentication is already done
    let state = MENDER_CLIENT_STATE.lock().await;
    if *state == MenderClientState::Authenticated {
        if let Err(e) = addon.activate().await {
            log_error!("Unable to activate add-on");
            // Cleanup on failure
            if let Err(e) = addon.exit().await {
                log_error!("Add-on exit failed: {:?}", e);
            }
            return Err(e);
        }
    }

    // Add add-on to the list using the trait object
    addons.push(addon as &'static dyn MenderAddon);

    Ok((MenderStatus::Ok, ()))
}

pub async fn mender_client_activate() -> MenderStatus {
    log_info!("mender_client_activate");
    let mut client_work = MENDER_CLIENT_WORK.lock().await;

    let work = match client_work.as_mut() {
        Some(w) => w,
        None => return MenderStatus::Other,
    };

    if mender_scheduler_work_activate(work).await.is_ok() {
        log_info!("mender_client_activate: update work activated");
        MenderStatus::Done
    } else {
        log_error!("Unable to activate update work");
        MenderStatus::Other
    }
}

async fn deactivate_addons() -> MenderResult<()> {
    let addons = MENDER_CLIENT_ADDONS.lock().await;

    // Deactivate each addon
    for addon in addons.iter() {
        if let Err(e) = addon.deactivate().await {
            log_error!("Failed to deactivate addon");
            return Err(e);
        }
    }

    Ok((MenderStatus::Ok, ()))
}

pub async fn mender_client_deactivate() -> MenderStatus {
    // Deactivate add-ons
    if let Err(e) = deactivate_addons().await {
        log_error!("Failed to deactivate addons");
        return e;
    }

    let mut client_work = MENDER_CLIENT_WORK.lock().await;

    let work = match client_work.as_mut() {
        Some(w) => w,
        None => return MenderStatus::Other,
    };

    if mender_scheduler_work_deactivate(work).await.is_ok() {
        MenderStatus::Done
    } else {
        log_error!("Unable to deactivate update work");
        MenderStatus::Other
    }
}

pub async fn mender_client_network_connect() -> MenderResult<()> {
    log_info!("mender_client_network_connect");
    let mut count = MENDER_CLIENT_NETWORK_COUNT.lock().await;

    // Check if this is the first network user
    if *count == 0 {
        // Request network access if callback exists
        let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
        if let Some(cb) = callbacks.as_ref() {
            (cb.network_connect)()?;
        }
    }

    // Increment network management counter
    *count += 1;

    Ok((MenderStatus::Ok, ()))
}

pub async fn mender_client_network_release() -> MenderResult<()> {
    let mut count = MENDER_CLIENT_NETWORK_COUNT.lock().await;

    // Decrement network management counter
    *count = count.saturating_sub(1);

    // Check if this was the last network user
    if *count == 0 {
        // Release network access if callback exists
        let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
        if let Some(cb) = callbacks.as_ref() {
            (cb.network_release)()?;
        }
    }

    Ok((MenderStatus::Ok, ()))
}

#[allow(dead_code)]
async fn release_addons() -> MenderResult<()> {
    let mut addons = MENDER_CLIENT_ADDONS.lock().await;

    // Release each addon
    for addon in addons.iter() {
        if let Err(e) = addon.exit().await {
            log_error!("Failed to exit addon");
            return Err(e);
        }
    }

    // Clear the addons list
    addons.clear();

    Ok((MenderStatus::Ok, ()))
}

#[allow(dead_code)]
pub async fn mender_client_exit() -> MenderStatus {
    // Release add-ons
    if let Err(e) = release_addons().await {
        log_error!("Failed to release addons");
        return e;
    }

    let mut client_work = MENDER_CLIENT_WORK.lock().await;

    if let Some(work) = client_work.take() {
        if mender_scheduler_work_delete(&work).await.is_ok() {
            log_info!("Update work deleted");
        } else {
            log_error!("Unable to delete update work");
        }
    }

    /* Release all modules */
    mender_api::mender_api_exit().await;
    if (mender_tls::mender_tls_exit().await).is_err() {
        log_error!("Unable to exit TLS");
        return MenderStatus::Failed;
    }
    let _ = mender_storage::mender_storage_exit().await;
    if (mender_scheduler::mender_scheduler_work_delete_all().await).is_err() {
        log_error!("Failed to delete all scheduler work");
        return MenderStatus::Failed;
    }

    MenderStatus::Done
}

// In your client code
async fn mender_client_work_function() -> MenderStatus {
    log_info!("mender_client_work_function");

    let mut state = MENDER_CLIENT_STATE.lock().await;
    if *state == MenderClientState::Initialization {
        // Perform initialization of the client
        match mender_client_initialization_work_function().await {
            Ok(_) => {
                // Update client state
                *state = MenderClientState::Authentication;
            }
            Err(e) => return e,
        }
    }

    match mender_client_network_connect().await {
        Ok(_) => (),
        Err(e) => return e,
    }

    // Intentional pass-through
    if *state == MenderClientState::Authentication {
        // Perform authentication with the server
        if let Err(e) = mender_client_authentication_work_function().await {
            if let Err(release_err) = mender_client_network_release().await {
                return release_err;
            }
            return e;
        }

        let period = {
            let config = MENDER_CLIENT_CONFIG.lock().await;
            config.as_ref().map(|c| c.update_poll_interval).unwrap_or(0)
        };

        let work_context = {
            let mut work = MENDER_CLIENT_WORK.lock().await;
            work.as_mut().cloned() // Clone the work context
        };

        if let Some(mut w) = work_context {
            log_info!(
                "mender_client_work_function: setting work period: {}",
                period
            );
            if (mender_scheduler::mender_scheduler_work_set_period(&mut w, period)).is_err() {
                log_error!("Unable to set work period");
                if let Err(release_err) = mender_client_network_release().await {
                    return release_err;
                }
                return MenderStatus::Other;
            }
        }

        log_info!("mender_client_work_function: setting work period done");
        // Update client state
        *state = MenderClientState::Authenticated;
    }

    /* Intentional pass-through */
    if *state == MenderClientState::Authenticated {
        // Perform updates
        mender_client_update_work_function().await
    } else {
        MenderStatus::Ok
    }
}

fn mender_client_work() -> MenderFuture {
    Box::pin(async {
        match mender_client_work_function().await {
            MenderStatus::Done => Ok(()),
            _ => Err("Work failed"),
        }
    })
}

async fn mender_client_initialization_work_function() -> MenderResult<()> {
    log_info!("mender_client_initialization_work_function");
    // Retrieve or generate authentication keys
    let config = MENDER_CLIENT_CONFIG.lock().await;
    let recommissioning = config.as_ref().map(|c| c.recommissioning).unwrap_or(false);

    let mut lock = MENDER_CLIENT_RNG.lock().await;
    let rng = lock.as_mut().ok_or(MenderStatus::Failed)?;

    mender_tls::mender_tls_init_authentication_keys(rng.get_trng(), recommissioning).await?;

    // Retrieve deployment data if it exists
    match mender_storage::mender_storage_get_deployment_data().await {
        Ok((_, deployment_data)) => {
            // Parse deployment data using from_str
            match from_str::<DeploymentData>(&deployment_data) {
                Ok((json_data, _)) => {
                    let mut deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
                    *deployment = Some(json_data);
                    log_info!("Successfully parsed deployment: {:?}", deployment);
                }
                Err(e) => {
                    log_error!("Failed to parse deployment data, error: {}", e);
                    mender_storage::mender_storage_delete_deployment_data().await?;

                    let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
                    if let Some(cb) = callbacks.as_ref() {
                        if let Err(e) = (cb.restart)() {
                            log_error!("Restart callback failed: {:?}", e);
                            return Err(e);
                        }
                    }
                    return Err(MenderStatus::Failed);
                }
            }
        }
        Err(MenderStatus::NotFound) => {
            log_info!("No deployment data found");
        }
        Err(e) => {
            log_error!("Failed to get deployment data, error: {:?}", e);
            mender_storage::mender_storage_delete_deployment_data().await?;

            let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
            if let Some(cb) = callbacks.as_ref() {
                if let Err(e) = (cb.restart)() {
                    log_error!("Restart callback failed: {:?}", e);
                    return Err(e);
                }
            }
            return Err(MenderStatus::Failed);
        }
    }

    Ok((MenderStatus::Done, ()))
}

pub struct MyDownLoad;

impl MenderCallback for MyDownLoad {
    fn call<'a>(
        &'a self,
        info: MenderCallbackInfo<'a>,
    ) -> Pin<Box<dyn Future<Output = MenderResult<()>> + Send + 'a>> {
        Box::pin(async move {
            mender_client_download_artifact_callback(
                info.type_str,
                info.meta,
                info.file,
                info.size,
                info.data,
                info.offset,
                info.total,
                info.chksum,
            )
            .await
        })
    }
}

async fn mender_client_update_work_function() -> MenderStatus {
    // Check for deployment
    log_info!("mender_client_update_work_function");
    let deployment = match mender_api::mender_api_check_for_deployment().await {
        Ok((_, (id, artifact_name, uri))) => {
            // Check if deployment is available
            if id.is_empty() || artifact_name.is_empty() || uri.is_empty() {
                log_info!("No deployment available");
                return MenderStatus::Done;
            }
            log_debug!(
                "Deployment available, id: {}, artifact_name: {}, uri: {}",
                id,
                artifact_name,
                uri
            );
            Some((id, artifact_name, uri))
        }
        Err(e) => {
            log_error!("Unable to check for deployment");
            return e;
        }
    };

    let (id, artifact_name, uri) = deployment.unwrap();
    // Unescape the URI - replace \u0026 with &
    let unescaped_uri = uri.replace("\\u0026", "&");

    // Reset flags
    MENDER_CLIENT_DEPLOYMENT_NEEDS_SET_PENDING_IMAGE.store(false, Ordering::SeqCst);
    MENDER_CLIENT_DEPLOYMENT_NEEDS_RESTART.store(false, Ordering::SeqCst);

    // Create deployment data structure
    let deployment_data = DeploymentData {
        id: id.to_string(),
        artifact_name: artifact_name.to_string(),
        types: Vec::new(), // Initialize with empty vector
    };

    {
        // Save to MENDER_CLIENT_DEPLOYMENT_DATA
        let mut deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
        *deployment = Some(deployment_data.clone());
    }
    // Download deployment artifact
    log_debug!(
        "Downloading deployment artifact with id: {}, artifact_name: {}, uri: {}",
        id,
        artifact_name,
        unescaped_uri
    );
    mender_client_publish_deployment_status(&id, DeploymentStatus::Downloading).await;

    let download_callback = MyDownLoad;

    match mender_api::mender_api_download_artifact(&unescaped_uri, Some(&download_callback)).await {
        Ok(_) => (),
        Err(e) => {
            log_error!("Unable to download artifact");
            mender_client_publish_deployment_status(&id, DeploymentStatus::Failure).await;
            {
                let mut deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
                *deployment = None;
            }
            let needs_set_pending_image =
                MENDER_CLIENT_DEPLOYMENT_NEEDS_SET_PENDING_IMAGE.load(Ordering::SeqCst);
            if needs_set_pending_image {
                match mender_flash::mender_flash_abort_deployment().await {
                    Ok(_) => (),
                    Err(e) => return e,
                }
            }
            return e;
        }
    }

    // Set boot partition
    log_info!("Download done, installing artifact");
    mender_client_publish_deployment_status(&id, DeploymentStatus::Installing).await;
    let needs_set_pending_image =
        MENDER_CLIENT_DEPLOYMENT_NEEDS_SET_PENDING_IMAGE.load(Ordering::SeqCst);
    if needs_set_pending_image {
        if let Err(e) = mender_flash::mender_flash_set_pending_image().await {
            log_error!("Unable to set boot partition");
            mender_client_publish_deployment_status(&id, DeploymentStatus::Failure).await;
            {
                let mut deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
                *deployment = None;
            }
            return e;
        }
    }

    // Handle restart case
    let needs_restart = MENDER_CLIENT_DEPLOYMENT_NEEDS_RESTART.load(Ordering::SeqCst);
    if needs_restart {
        // Save deployment data
        let deployment_str: HString<256> = {
            let deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
            if let Some(deployment) = deployment.as_ref() {
                match serde_json_core::to_string(&deployment) {
                    Ok(str) => str,
                    Err(_) => {
                        log_error!("Failed to serialize deployment data");
                        return MenderStatus::Failed;
                    }
                }
            } else {
                log_error!("No deployment data available");
                return MenderStatus::Failed;
            }
        };

        match mender_storage::mender_storage_set_deployment_data(&deployment_str).await {
            Ok(_) => (),
            Err(e) => {
                log_error!("Unable to save deployment data");
                mender_client_publish_deployment_status(&id, DeploymentStatus::Failure).await;
                {
                    let mut deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
                    *deployment = None;
                }
                return e;
            }
        }
        mender_client_publish_deployment_status(&id, DeploymentStatus::Rebooting).await;

        {
            let mut deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
            *deployment = None;
        }

        // Get callbacks and trigger restart
        let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
        if let Some(cb) = callbacks.as_ref() {
            match (cb.restart)() {
                Ok(_) => (),
                Err(e) => return e,
            }
        }
    } else {
        // Publish success if no restart needed
        mender_client_publish_deployment_status(&id, DeploymentStatus::Success).await;
        {
            let mut deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
            *deployment = None;
        }
    }

    MenderStatus::Done
}

async fn mender_client_publish_deployment_status(
    id: &str,
    status: DeploymentStatus,
) -> MenderStatus {
    log_info!(
        "mender_client_publish_deployment_status, id: {}, status: {}",
        id,
        status
    );
    // Publish status to the mender server
    let ret = mender_api::mender_api_publish_deployment_status(id, status).await;

    // Invoke deployment status callback if defined
    let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
    if let Some(cb) = callbacks.as_ref() {
        let _ = (cb.deployment_status)(status, Some(status.as_str()));
    }

    match ret {
        Ok(_) => MenderStatus::Done,
        Err(e) => e,
    }
}

#[allow(clippy::too_many_arguments)]
async fn mender_client_download_artifact_callback(
    artifact_type: Option<&str>,
    meta_data: Option<&str>,
    filename: Option<&str>,
    size: u32,
    data: &[u8],
    index: u32,
    length: u32,
    chksum: &[u8],
) -> MenderResult<()> {
    log_debug!(
        "mender_client_download_artifact_callback, size: {}, index: {}, length: {}",
        size,
        index,
        length
    );

    let artifact_types = MENDER_CLIENT_ARTIFACT_TYPES.lock().await;

    // Check if we have any registered types
    if let Some(types_list) = artifact_types.as_ref() {
        //log_info!("mender_client_download_artifact_callback: types_list");
        // Look for matching type handler
        for artifact_handler in types_list.iter() {
            //log_info!("mender_client_download_artifact_callback: artifact_handler", "artifact_handler.type_name" => artifact_handler.type_name);
            if let Some(artifact_type_str) = artifact_type {
                if artifact_handler.type_name == artifact_type_str {
                    // Invoke callback for the artifact type
                    let _meta_data_str = meta_data.unwrap_or("");
                    (artifact_handler.callback)
                        .call(
                            // id,
                            // artifact_name,
                            // artifact_type_str,
                            // meta_data_str,
                            filename.unwrap_or(""),
                            size,
                            data,
                            index,
                            length,
                            chksum,
                        )
                        .await?;

                    // Handle first chunk special case
                    if index == 0 {
                        log_debug!(
                            "Adding artifact type {} to the deployment data",
                            artifact_type_str
                        );

                        // Convert the type string
                        let type_str = artifact_type_str.to_string();

                        // Check if we need to add the type and add it in a smaller scope
                        {
                            let mut deployment_data = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
                            let deployment = deployment_data.as_mut().unwrap();

                            if deployment.types.is_empty()
                                || !deployment.types.iter().any(|t| t == &type_str)
                            {
                                log_debug!(
                                    "Adding artifact type {} to the deployment data",
                                    artifact_type_str
                                );
                                deployment.types.push(type_str);
                            }
                        }

                        // Set restart flag if needed
                        if artifact_handler.needs_restart {
                            MENDER_CLIENT_DEPLOYMENT_NEEDS_RESTART.store(true, Ordering::SeqCst);
                        }
                    }

                    return Ok((MenderStatus::Ok, ()));
                }
            }
        }
    }

    // No matching handler found
    log_error!(
        "Unable to handle artifact type: {}",
        artifact_type.unwrap_or("")
    );
    Err(MenderStatus::Failed)
}

async fn mender_client_authentication_work_function() -> MenderResult<()> {
    log_info!("mender_client_authentication_work_function");
    // Perform authentication with the mender server
    if let Err(e) = mender_api::mender_api_perform_authentication().await {
        // Invoke authentication error callback
        let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
        if let Some(cb) = callbacks.as_ref() {
            if (cb.authentication_failure)().is_err() {
                // Check if deployment is pending
                let deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
                if deployment.is_some() {
                    log_error!("Authentication error callback failed, rebooting");
                    // Invoke restart callback
                    (cb.restart)()?;
                }
            }
        }
        return Err(e);
    }

    {
        // Invoke authentication success callback
        let callbacks = MENDER_CLIENT_CALLBACKS.lock().await;
        if let Some(cb) = callbacks.as_ref() {
            if (cb.authentication_success)().is_err() {
                // Check if deployment is pending
                let deployment = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
                if deployment.is_some() {
                    log_error!("Authentication success callback failed, rebooting");
                    (cb.restart)()?;
                }
            }
        }
    }

    // Check if deployment is pending
    let mut deployment_data = MENDER_CLIENT_DEPLOYMENT_DATA.lock().await;
    if let Some(deployment) = deployment_data.as_ref() {
        // Get deployment ID and artifact name
        let id = &deployment.id;
        let artifact_name = &deployment.artifact_name;
        let types = &deployment.types;

        // Check if artifact running is the pending one
        let mut success = true;
        let artifact_types = MENDER_CLIENT_ARTIFACT_TYPES.lock().await;

        if let Some(type_list) = artifact_types.as_ref() {
            for deployment_type in types.iter() {
                for artifact_type in type_list.iter() {
                    if artifact_type.type_name == *deployment_type
                        && artifact_type.artifact_name != *artifact_name
                    {
                        success = false;
                    }
                }
            }
        }

        // Publish deployment status
        if success {
            mender_client_publish_deployment_status(id, DeploymentStatus::Success).await;
        } else {
            mender_client_publish_deployment_status(id, DeploymentStatus::Failure).await;
        }

        // Delete pending deployment
        mender_storage::mender_storage_delete_deployment_data().await?;
    }

    // Clear deployment data
    *deployment_data = None;

    // Activate add-ons after successful authentication
    if let Err(e) = activate_addons().await {
        log_error!("Failed to activate addons");
        return Err(e);
    }

    Ok((MenderStatus::Done, ()))
}

async fn activate_addons() -> MenderResult<()> {
    log_info!("activate_addons");
    let addons = MENDER_CLIENT_ADDONS.lock().await;

    // Activate each addon
    for addon in addons.iter() {
        if let Err(e) = addon.activate().await {
            log_error!("Failed to activate addon");
            return Err(e);
        }
    }

    Ok((MenderStatus::Ok, ()))
}

async fn mender_client_download_artifact_flash_callback(
    // _id: &str,
    // _artifact_name: &str,
    // _type_name: &str,
    // _meta_data: &str,
    filename: &str,
    size: u32,
    data: &[u8],
    index: u32,
    length: u32,
    chksum: &[u8],
) -> MenderResult<()> {
    log_info!("mender_client_download_artifact_flash_callback, filename: {}, size: {}, index: {}, length: {}", filename, size, index, length);

    // Only proceed if filename is not empty
    if !filename.is_empty() {
        // Open flash handle if this is the first chunk
        if index == 0 {
            match mender_flash::mender_flash_open(filename, size, chksum).await {
                Ok(_) => (),
                Err(e) => {
                    log_error!(
                        "Unable to open flash handle, filename: {}, size: {}",
                        filename,
                        size
                    );
                    return Err(e);
                }
            }
        }

        // Write data to flash
        if let Err(e) = mender_flash::mender_flash_write(data, index, length).await {
            log_error!(
                "Unable to write data to flash, filename: {}, size: {}, index: {}, length: {}",
                filename,
                size,
                index,
                length
            );
            return Err(e);
        }

        // Close flash handle if this is the last chunk
        if index + length >= size {
            if let Err(e) = mender_flash::mender_flash_close().await {
                log_error!(
                    "Unable to close flash handle, filename: {}, size: {}",
                    filename,
                    size
                );
                return Err(e);
            }
        }
    }

    // Set pending image flag
    MENDER_CLIENT_DEPLOYMENT_NEEDS_SET_PENDING_IMAGE.store(true, Ordering::SeqCst);

    Ok((MenderStatus::Ok, ()))
}
