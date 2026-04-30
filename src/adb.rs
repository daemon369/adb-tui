use anyhow::{anyhow, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub status: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// Find ADB executable path - cross-platform detection
pub fn find_adb_path() -> Result<String> {
    // First, try to run adb directly - works if it's in PATH
    if cfg!(target_os = "windows") {
        let output = Command::new("where")
            .arg("adb")
            .output();
        
        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // where can return multiple lines, take the first one
                let path = stdout.lines().next().unwrap_or("").trim().to_string();
                if !path.is_empty() && Path::new(&path).exists() {
                    return Ok(path);
                }
            }
        }
    } else {
        // Linux/macOS - check if adb is in PATH
        let output = Command::new("which")
            .arg("adb")
            .output();
        
        if let Ok(output) = output {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(path);
                }
            }
        }
        
        // WSL - try to run Windows ADB via cmd.exe
        if Path::new("/mnt/c/Windows/System32/cmd.exe").exists() {
            let output = Command::new("/mnt/c/Windows/System32/cmd.exe")
                .args(["/c", "where", "adb"])
                .output();
            
            if let Ok(output) = output {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let path = stdout.lines().next().unwrap_or("").trim().to_string();
                    if !path.is_empty() {
                        // Convert Windows path to WSL path
                        let wsl_path = windows_to_wsl_path(&path);
                        if Path::new(&wsl_path).exists() {
                            return Ok(wsl_path);
                        }
                    }
                }
            }
        }
    }
    
    // Try common installation paths
    let common_paths = get_common_adb_paths();
    for path in &common_paths {
        if Path::new(path).exists() {
            return Ok(path.clone());
        }
    }
    
    Err(anyhow!("ADB executable not found. Please ensure Android SDK Platform-Tools is installed and 'adb' is in your PATH"))
}

/// Convert Windows path to WSL path
fn windows_to_wsl_path(windows_path: &str) -> String {
    if windows_path.contains(':') {
        let parts: Vec<&str> = windows_path.split(':').collect();
        if parts.len() >= 2 {
            let drive = parts[0].to_lowercase();
            let rest = parts[1].replace('\\', "/");
            return format!("/mnt/{}{}", drive, rest);
        }
    }
    windows_path.to_string()
}

/// Run ADB command - handles WSL/Windows cross-platform execution
fn run_adb_command(adb_path: &str, args: &[&str]) -> Result<std::process::Output> {
    // If running in WSL and adb_path is a Windows executable
    if cfg!(target_os = "linux") && adb_path.ends_with(".exe") {
        // Use cmd.exe to run Windows ADB from WSL
        let mut cmd_args = vec!["/c", adb_path];
        cmd_args.extend_from_slice(args);
        
        let output = Command::new("/mnt/c/Windows/System32/cmd.exe")
            .args(&cmd_args)
            .output()?;
        
        return Ok(output);
    }
    
    // Normal execution
    let output = Command::new(adb_path)
        .args(args)
        .output()?;
    
    Ok(output)
}

fn get_common_adb_paths() -> Vec<String> {
    let mut paths = Vec::new();
    
    if cfg!(target_os = "windows") {
        if let Ok(home) = std::env::var("USERPROFILE") {
            paths.push(format!("{}\\AppData\\Local\\Android\\sdk\\platform-tools\\adb.exe", home));
            paths.push(format!("{}\\Android\\sdk\\platform-tools\\adb.exe", home));
        }
        paths.push("C:\\Android\\sdk\\platform-tools\\adb.exe".to_string());
        paths.push("C:\\Program Files\\Android\\platform-tools\\adb.exe".to_string());
        paths.push("C:\\Program Files (x86)\\Android\\platform-tools\\adb.exe".to_string());
    } else if cfg!(target_os = "macos") {
        if let Ok(home) = std::env::var("HOME") {
            paths.push(format!("{}/Library/Android/sdk/platform-tools/adb", home));
        }
        paths.push("/usr/local/bin/adb".to_string());
        paths.push("/opt/homebrew/bin/adb".to_string());
    } else {
        // Linux - check both Linux and Windows (WSL) paths
        if let Ok(home) = std::env::var("HOME") {
            paths.push(format!("{}/Android/Sdk/platform-tools/adb", home));
        }
        paths.push("/usr/bin/adb".to_string());
        paths.push("/usr/local/bin/adb".to_string());
        
        // WSL - check Windows paths
        if std::path::Path::new("/mnt/c").exists() {
            // Try to get Windows username
            if let Ok(entries) = std::fs::read_dir("/mnt/c/Users") {
                for entry in entries.flatten() {
                    if let Some(user) = entry.file_name().to_str() {
                        paths.push(format!("/mnt/c/Users/{}/AppData/Local/Android/sdk/platform-tools/adb.exe", user));
                        paths.push(format!("/mnt/c/Users/{}/Android/sdk/platform-tools/adb.exe", user));
                    }
                }
            }
            paths.push("/mnt/c/Android/sdk/platform-tools/adb.exe".to_string());
            paths.push("/mnt/c/Program Files/Android/platform-tools/adb.exe".to_string());
        }
    }
    
    paths
}

pub fn check_adb_available() -> Result<String> {
    ensure_adb_server_running()?;
    check_adb_server_running()
}

pub fn get_devices() -> Result<Vec<Device>> {
    ensure_adb_server_running()?;
    get_devices_socket()
}

fn get_device_model(adb_path: &str, device_id: &str) -> Option<String> {
    let output = run_adb_command(adb_path, &["-s", device_id, "shell", "getprop", "ro.product.model"]).ok()?;

    let model = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if model.is_empty() {
        None
    } else {
        Some(model)
    }
}

pub fn execute_command(device_id: &str, command: &str) -> Result<(String, String)> {
    ensure_adb_server_running()?;
    execute_command_socket(device_id, command)
}

pub fn batch_execute(devices: &[String], command: &str) -> Vec<(String, Result<(String, String)>)> {
    devices
        .iter()
        .map(|device_id| {
            let result = execute_command(device_id, command);
            (device_id.clone(), result)
        })
        .collect()
}

pub fn pull_file(device_id: &str, remote_path: &str, local_path: &str) -> Result<()> {
    let adb_path = find_adb_path()?;
    
    let output = run_adb_command(&adb_path, &["-s", device_id, "pull", remote_path, local_path])?;
    
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to pull file from device"))
    }
}

pub fn push_file(device_id: &str, local_path: &str, remote_path: &str) -> Result<()> {
    let adb_path = find_adb_path()?;
    
    let output = run_adb_command(&adb_path, &["-s", device_id, "push", local_path, remote_path])?;
    
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to push file to device"))
    }
}

pub fn batch_pull(devices: &[String], remote_path: &str, local_dir: &str) -> Vec<(String, Result<()>)> {
    devices
        .iter()
        .map(|device_id| {
            let device_local_dir = format!("{}/{}", local_dir, device_id);
            std::fs::create_dir_all(&device_local_dir).ok();
            let result = pull_file(device_id, remote_path, &device_local_dir);
            (device_id.clone(), result)
        })
        .collect()
}

pub fn batch_push(devices: &[String], local_path: &str, remote_path: &str) -> Vec<(String, Result<()>)> {
    devices
        .iter()
        .map(|device_id| {
            let result = push_file(device_id, local_path, remote_path);
            (device_id.clone(), result)
        })
        .collect()
}

pub fn run_binary(device_id: &str, binary_path: &str, args: &[&str]) -> Result<(String, String)> {
    ensure_adb_server_running()?;

    let full_command = if args.is_empty() {
        binary_path.to_string()
    } else {
        format!("{} {}", binary_path, args.join(" "))
    };

    execute_command_socket(device_id, &full_command)
}

pub fn list_files(device_id: &str, path: &str) -> Result<Vec<FileEntry>> {
    let adb_path = find_adb_path()?;

    let output = run_adb_command(&adb_path, &["-s", device_id, "shell", "ls", "-la", path])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();

    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 7 {
            let name = parts[8..].join(" ");
            let is_dir = line.starts_with('d');
            let path_full = format!("{}/{}", path.trim_end_matches('/'), name);
            files.push(FileEntry {
                name,
                path: path_full,
                is_dir,
            });
        }
    }

    Ok(files)
}

// ========== Socket-based ADB communication ==========

const ADB_HOST: &str = "127.0.0.1";
const ADB_PORT: u16 = 5037;

/// Connect to ADB server via socket
fn connect_adb() -> Result<TcpStream> {
    let stream = TcpStream::connect((ADB_HOST, ADB_PORT))
        .map_err(|e| anyhow!("Failed to connect to ADB server at {}:{}: {}", ADB_HOST, ADB_PORT, e))?;
    Ok(stream)
}

/// Send ADB command and read response
fn adb_socket_command(command: &str) -> Result<String> {
    let mut stream = connect_adb()?;

    // ADB protocol: send length in hex (4 bytes) followed by the command
    let cmd_len = format!("{:04x}{}", command.len(), command);
    stream.write_all(cmd_len.as_bytes())?;

    // Read response: first 4 bytes indicate status or length
    let mut response_header = [0u8; 4];
    stream.read_exact(&mut response_header)?;

    let header = String::from_utf8_lossy(&response_header);

    if header == "OKAY" {
        // Read length of data to follow
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len_str = String::from_utf8_lossy(&len_buf);
        let len = usize::from_str_radix(len_str.trim(), 16)
            .map_err(|e| anyhow!("Invalid length in ADB response: {}", e))?;

        if len > 0 {
            let mut data = vec![0u8; len];
            stream.read_exact(&mut data)?;
            Ok(String::from_utf8_lossy(&data).to_string())
        } else {
            Ok(String::new())
        }
    } else if header == "FAIL" {
        // Read error message
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len_str = String::from_utf8_lossy(&len_buf);
        let len = usize::from_str_radix(len_str.trim(), 16)
            .map_err(|e| anyhow!("Invalid length in ADB response: {}", e))?;

        let mut data = vec![0u8; len];
        stream.read_exact(&mut data)?;
        Err(anyhow!("ADB command failed: {}", String::from_utf8_lossy(&data)))
    } else {
        // Could be a direct response (like from "host:devices" or "host:version")
        // Read remaining data
        let mut data = vec![0u8; 4];
        stream.read_exact(&mut data)?;
        let len_str = String::from_utf8_lossy(&data);
        let len = usize::from_str_radix(len_str.trim(), 16)
            .map_err(|e| anyhow!("Invalid length in ADB response: {}", e))?;

        if len > 0 {
            let mut response_data = vec![0u8; len];
            stream.read_exact(&mut response_data)?;
            Ok(String::from_utf8_lossy(&response_data).to_string())
        } else {
            Ok(header.to_string())
        }
    }
}

/// Get ADB server version via socket
pub fn get_adb_version_socket() -> Result<String> {
    let response = adb_socket_command("host:version")?;
    Ok(format!("ADB server version: {}", response))
}

/// Get list of devices via socket connection to ADB server
pub fn get_devices_socket() -> Result<Vec<Device>> {
    let response = adb_socket_command("host:devices-l")?;

    let mut devices = Vec::new();

    for line in response.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && (parts[1] == "device" || parts[1] == "unauthorized" || parts[1] == "offline") {
            let id = parts[0].to_string();
            let status = parts[1].to_string();

            let mut device = Device {
                id,
                status,
                model: None,
            };

            if device.status == "device" {
                if let Some(model) = get_device_model_socket(&device.id)? {
                    device.model = Some(model);
                }
            }

            devices.push(device);
        }
    }

    Ok(devices)
}

/// Get device property via socket
fn get_device_model_socket(device_id: &str) -> Result<Option<String>> {
    let command = format!("host-serial:{}:getprop:ro.product.model", device_id);
    match adb_socket_command(&command) {
        Ok(response) => {
            let model = response.trim().to_string();
            if model.is_empty() {
                Ok(None)
            } else {
                Ok(Some(model))
            }
        }
        Err(_) => Ok(None),
    }
}

/// Execute shell command on device via socket
pub fn execute_command_socket(device_id: &str, command: &str) -> Result<(String, String)> {
    let mut stream = connect_adb()?;

    // First, select the device by sending transport command
    if !device_id.is_empty() {
        let transport_cmd = format!("host:transport:{}", device_id);
        let cmd_len = format!("{:04x}{}", transport_cmd.len(), transport_cmd);
        stream.write_all(cmd_len.as_bytes())?;

        // Read response header
        let mut response_header = [0u8; 4];
        stream.read_exact(&mut response_header)?;
        let header = String::from_utf8_lossy(&response_header);

        if header != "OKAY" {
            return Err(anyhow!("Failed to select device: {}", header));
        }
    }

    // Now send the shell command
    let shell_cmd = format!("shell:{}", command);
    let cmd_len = format!("{:04x}{}", shell_cmd.len(), shell_cmd);
    stream.write_all(cmd_len.as_bytes())?;

    // Read response header
    let mut response_header = [0u8; 4];
    stream.read_exact(&mut response_header)?;
    let header = String::from_utf8_lossy(&response_header);

    if header != "OKAY" {
        return Err(anyhow!("Failed to execute shell command: {}", header));
    }

    // Read output until connection closes
    let mut stdout = String::new();
    match stream.read_to_string(&mut stdout) {
        Ok(_) => Ok((stdout, String::new())),
        Err(e) => Err(anyhow!("Error reading command output: {}", e)),
    }
}

/// Check if ADB server is running via socket
pub fn check_adb_server_running() -> Result<String> {
    match connect_adb() {
        Ok(_) => {
            let version = get_adb_version_socket()?;
            Ok(version)
        }
        Err(e) => Err(anyhow!("ADB server not running: {}", e)),
    }
}

/// Start ADB server if not running
pub fn ensure_adb_server_running() -> Result<()> {
    if connect_adb().is_err() {
        // Try to start ADB server
        let adb_path = find_adb_path()?;
        let output = run_adb_command(&adb_path, &["start-server"])?;
        if !output.status.success() {
            return Err(anyhow!("Failed to start ADB server"));
        }
    }
    Ok(())
}

/// Listen for device changes using ADB track-devices feature
/// This function runs in a background thread and sends device updates via the channel
pub fn track_devices(tx: std::sync::mpsc::Sender<Vec<Device>>) {
    loop {
        // Ensure ADB server is running
        if ensure_adb_server_running().is_err() {
            // Wait before retrying
            std::thread::sleep(std::time::Duration::from_secs(3));
            continue;
        }

        // Connect to ADB server
        let mut stream = match connect_adb() {
            Ok(s) => s,
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_secs(3));
                continue;
            }
        };

        // Send track-devices command
        let cmd = "host:track-devices";
        let cmd_len = format!("{:04x}{}", cmd.len(), cmd);
        if stream.write_all(cmd_len.as_bytes()).is_err() {
            std::thread::sleep(std::time::Duration::from_secs(3));
            continue;
        }

        // Read initial response
        let mut response_header = [0u8; 4];
        if stream.read_exact(&mut response_header).is_err() {
            std::thread::sleep(std::time::Duration::from_secs(3));
            continue;
        }

        let header = String::from_utf8_lossy(&response_header);
        if header != "OKAY" {
            std::thread::sleep(std::time::Duration::from_secs(3));
            continue;
        }

        // Read initial device list
        if let Ok(devices) = read_track_response(&mut stream) {
            let _ = tx.send(devices);
        }

        // Now loop reading updates - the connection stays open
        loop {
            match read_track_response(&mut stream) {
                Ok(devices) => {
                    let _ = tx.send(devices);
                }
                Err(_) => {
                    // Connection lost, reconnect
                    break;
                }
            }
        }

        // Wait before reconnecting
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

/// Read a track-devices response
fn read_track_response(stream: &mut TcpStream) -> Result<Vec<Device>> {
    // Read length of data
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len_str = String::from_utf8_lossy(&len_buf);
    let len = usize::from_str_radix(len_str.trim(), 16)
        .map_err(|e| anyhow!("Invalid length in ADB response: {}", e))?;

    if len > 0 {
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data)?;
        let response = String::from_utf8_lossy(&data).to_string();
        parse_device_list(&response)
    } else {
        Ok(Vec::new())
    }
}

/// Parse device list from ADB response
fn parse_device_list(response: &str) -> Result<Vec<Device>> {
    let mut devices = Vec::new();

    for line in response.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && (parts[1] == "device" || parts[1] == "unauthorized" || parts[1] == "offline") {
            let id = parts[0].to_string();
            let status = parts[1].to_string();

            devices.push(Device {
                id,
                status,
                model: None,
            });
        }
    }

    Ok(devices)
}
