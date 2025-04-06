# 🛰️ Telematic Platform for Robotics, EV and IoT

## 📋 Overview

This open-source Telematic Platform for Robotics, EV, and IoT is designed for collecting, processing, and transmitting CAN bus data over multiple connectivity options. It enables real-time monitoring, remote control, and OTA updates for ECUs, making it ideal for robotics, electric vehicles, and IoT applications.

This project is open for contributions! If you're passionate about embedded systems, IoT, telematics, or robotics, we welcome you to collaborate, improve, and extend the platform.

## ✨ Key Features

| Feature                           | Description                                              |
| --------------------------------- | -------------------------------------------------------- |
| 🔄 **CAN Bus Integration**        | Collect and transmit CAN messages to cloud servers       |
| ⚙️ **Remote Configuration**       | Configure data rates, schedules, and commands remotely   |
| 📡 **OTA Updates**                | Update firmware for ECUs wirelessly via CAN              |
| 🌍 **GPS/GNSS Tracking**          | Real-time geolocation support                            |
| 🎮 **Remote Command & Control**   | Manage ECUs from the cloud                               |
| 📶 **Multi-Network Connectivity** | Supports Wi-Fi, Bluetooth, and LTE                       |
| 📊 **IMU Sensor Integration**     | Track vibration and environmental factors                |
| 🚨 **Safety Monitoring**          | Fall & crash detection for accidents and system failures |
| 🔍 **Remote Diagnostics**         | Remote fault analysis and debugging                      |

## 📸 Hardware Overview

<img width="259" alt="Telematic Platform Hardware" src="https://github.com/user-attachments/assets/8cb6f342-93dc-4081-9f0b-baa21884126f" />

_ESP32-C6 based hardware platform with CAN bus interface, multiple connectivity options, and sensor integrations_

## 🔧 Technical Specifications

| **Core Capabilities**         | **Technical Components**                |
| ----------------------------- | --------------------------------------- |
| 📡 Multi-Network Connectivity | Wi-Fi 6, BLE 5.3, LTE-M via ESP32-C6    |
| 🎛️ CAN Bus Integration        | ISO 15765-2 (CAN FD) with 5Mbps support |
| 🔄 OTA Updates                | Secure A/B partitioning with Mender.io  |
| 📍 GNSS Tracking              | Multi-constellation GPS/Galileo/GLONASS |
| 🚨 Safety Monitoring          | IMU-based crash detection (MPU-6050)    |
| 📊 Remote Diagnostics         | J1939/OBD-II protocol decoding          |

## 🏗️ Project Architecture

```
src/
├── app/              # Application logic
├── cfg/              # Configuration
├── hal/              # Hardware abstraction
├── svc/              # Reusable services
├── task/             # Async/concurrent tasks
├── util/             # Utilities
└── main.rs           # Entry point (initializes hardware, starts tasks)
```

## 🚀 Getting Started

### Prerequisites

- 📦 **Rust Toolchain** (`rustup`)
- 🛠 **ESP-IDF for Rust** (`espup`)
- 🔌 **ESP32 Development Board**
- 🌐 **Mender Server Account** (Hosted or Open Source)

### Environment Configuration

Set the following environment variables:

```shell
WIFI_SSID=your_wifi_network
WIFI_PSWD=your_wifi_password
MENDER_CLIENT_URL=your_mender_url
MENDER_CLIENT_TENANT_TOKEN=your_token  # optional
```

## 🔨 Installation & Setup

### Install Rust for ESP32

```bash
rustup install stable
cargo install espup
espup install
cargo install espflash
```

### Compile and Run

#### Without OTA

```bash
cargo run --release
```

#### With OTA

```bash
cargo run --release --feature ota
```

## 🤝 Community

### Join the Discussion

Please join us on Discord: https://discord.gg/b7vk6fza

### Demo

- Coming Soon
