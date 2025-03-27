use super::mender_inventory_api;
use crate::cfg::mender_cfg::CONFIG_MENDER_CLIENT_INVENTORY_REFRESH_INTERVAL;
use crate::mender_mcu_client::addon::mender_addon::MenderAddonInstance;
use crate::mender_mcu_client::core::mender_client;
use crate::mender_mcu_client::core::mender_utils::{KeyStore, MenderResult, MenderStatus};
use crate::mender_mcu_client::platform::scheduler::mender_scheduler::{
    self, MenderFuture, MenderSchedulerWorkContext,
};
#[allow(unused_imports)]
use crate::{log_debug, log_error, log_info, log_warn};
use alloc::boxed::Box;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

pub struct MenderInventoryConfig {
    pub refresh_interval: u32,
}

// Global static variables
static MENDER_INVENTORY_CONFIG: Mutex<CriticalSectionRawMutex, Option<MenderInventoryConfig>> =
    Mutex::new(None);
static MENDER_INVENTORY_KEYSTORE: Mutex<CriticalSectionRawMutex, Option<KeyStore>> =
    Mutex::new(None);
static MENDER_INVENTORY_WORK_HANDLE: Mutex<
    CriticalSectionRawMutex,
    Option<MenderSchedulerWorkContext>,
> = Mutex::new(None);

// Example implementation for a specific add-on:
pub const MENDER_INVENTORY_ADDON_INSTANCE: MenderAddonInstance<MenderInventoryConfig, ()> =
    MenderAddonInstance {
        init: |config, callbacks| {
            Box::pin(mender_inventory_init(
                config.map(|c| c as &MenderInventoryConfig),
                callbacks.map(|_| ()),
            ))
        },
        activate: || Box::pin(mender_inventory_activate()),
        deactivate: || Box::pin(mender_inventory_deactivate()),
        exit: || Box::pin(mender_inventory_exit()),
    };

pub async fn mender_inventory_init(
    config: Option<&MenderInventoryConfig>,
    _callbacks: Option<()>,
) -> MenderResult<()> {
    // Save configuration
    let mut conf = MENDER_INVENTORY_CONFIG.lock().await;
    *conf = Some(MenderInventoryConfig {
        refresh_interval: if let Some(cfg) = config {
            if cfg.refresh_interval != 0 {
                cfg.refresh_interval
            } else {
                CONFIG_MENDER_CLIENT_INVENTORY_REFRESH_INTERVAL
            }
        } else {
            CONFIG_MENDER_CLIENT_INVENTORY_REFRESH_INTERVAL
        },
    });

    let mut work_handle = MENDER_INVENTORY_WORK_HANDLE.lock().await;
    match mender_scheduler::mender_scheduler_work_create(
        mender_inventory_work,
        conf.as_ref().unwrap().refresh_interval,
        "mender_inventory",
    ) {
        Ok(handle) => {
            *work_handle = Some(handle);
            Ok((MenderStatus::Ok, ()))
        }
        Err(_) => {
            log_error!("Unable to create inventory work");
            Err(MenderStatus::Failed)
        }
    }
}

fn mender_inventory_work() -> MenderFuture {
    Box::pin(async {
        match mender_inventory_work_function().await {
            Ok(_) => Ok(()), // Discard the status, just return success
            Err(_) => Err("Inventory work failed"),
        }
    })
}

pub async fn mender_inventory_activate() -> MenderResult<()> {
    log_info!("mender_inventory_activate");
    let mut work_handle = MENDER_INVENTORY_WORK_HANDLE.lock().await;
    if let Some(handle) = work_handle.as_mut() {
        mender_scheduler::mender_scheduler_work_activate(handle)
            .await
            .map(|()| (MenderStatus::Ok, ()))
            .map_err(|_| {
                log_error!("Unable to activate inventory work");
                MenderStatus::Failed
            })
    } else {
        log_error!("Unable to activate inventory work");
        Err(MenderStatus::Failed)
    }
}

pub async fn mender_inventory_deactivate() -> MenderResult<()> {
    let mut work_handle = MENDER_INVENTORY_WORK_HANDLE.lock().await;
    if let Some(handle) = work_handle.as_mut() {
        mender_scheduler::mender_scheduler_work_deactivate(handle)
            .await
            .map_err(|_| {
                log_error!("Unable to deactivate inventory work");
                MenderStatus::Failed
            })?;
    }
    Ok((MenderStatus::Ok, ()))
}

pub async fn mender_inventory_set(inventory: &KeyStore) -> MenderResult<()> {
    let mut keystore = MENDER_INVENTORY_KEYSTORE.lock().await;

    // Release previous inventory
    *keystore = None;

    // Copy the new inventory
    *keystore = Some(inventory.clone());

    Ok((MenderStatus::Ok, ()))
}

#[allow(dead_code)]
pub async fn mender_inventory_execute() -> MenderResult<()> {
    let mut work_handle = MENDER_INVENTORY_WORK_HANDLE.lock().await;
    if let Some(handle) = work_handle.as_mut() {
        mender_scheduler::mender_scheduler_work_execute(handle)
            .await
            .map(|()| (MenderStatus::Ok, ()))
            .map_err(|_| {
                log_error!("Unable to trigger inventory work");
                MenderStatus::Failed
            })
    } else {
        Err(MenderStatus::Failed)
    }
}

pub async fn mender_inventory_exit() -> MenderResult<()> {
    // Delete mender inventory work
    let mut work_handle = MENDER_INVENTORY_WORK_HANDLE.lock().await;
    if let Some(handle) = work_handle.as_mut() {
        mender_scheduler::mender_scheduler_work_delete(handle)
            .await
            .map_err(|_| {
                log_error!("Unable to delete inventory work");
                MenderStatus::Failed
            })?;
    }
    *work_handle = None;

    // Release memory
    let mut config = MENDER_INVENTORY_CONFIG.lock().await;
    *config = None;

    let mut keystore = MENDER_INVENTORY_KEYSTORE.lock().await;
    *keystore = None;

    Ok((MenderStatus::Ok, ()))
}

async fn mender_inventory_work_function() -> MenderResult<()> {
    log_info!("mender_inventory_work_function");
    let keystore = MENDER_INVENTORY_KEYSTORE.lock().await;

    // Request access to the network
    if mender_client::mender_client_network_connect()
        .await
        .is_err()
    {
        log_error!("Requesting access to the network failed");
        return Err(MenderStatus::Failed);
    }

    // Get artifact name and device type first since they're async
    let artifact_name = mender_client::mender_client_get_artifact_name().await;
    let device_type = mender_client::mender_client_get_device_type().await;

    // Convert KeyStore to slice if present
    let inventory_slice = keystore.as_ref().map(core::slice::from_ref);

    // Publish inventory
    if mender_inventory_api::mender_inventory_api_publish_inventory_data(
        artifact_name.as_deref(), // Convert Option<String> to Option<&str>
        device_type.as_deref(),   // Convert Option<String> to Option<&str>
        inventory_slice,          // Pass as Option<&[KeyStore]>
    )
    .await
    .is_err()
    {
        log_error!("Unable to publish inventory data");
    }

    // Release access to the network
    mender_client::mender_client_network_release().await?;

    Ok((MenderStatus::Ok, ()))
}
