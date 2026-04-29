use anyhow::{anyhow, Result};
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
    let adb_path = find_adb_path()?;
    
    let output = run_adb_command(&adb_path, &["version"])?;
    
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(version)
    } else {
        Err(anyhow!("ADB found at {} but failed to run", adb_path))
    }
}

pub fn get_devices() -> Result<Vec<Device>> {
    let adb_path = find_adb_path()?;
    
    let output = run_adb_command(&adb_path, &["devices", "-l"])?;
    
    if !output.status.success() {
        return Err(anyhow!("Failed to execute adb devices"));
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    
    let mut devices = Vec::new();
    
    for (idx, line) in stdout.lines().enumerate() {
        if idx == 0 {
            continue;
        }
        
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
                if let Some(model) = get_device_model(&adb_path, &device.id) {
                    device.model = Some(model);
                }
            }
            
            devices.push(device);
        }
    }
    
    Ok(devices)
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
    let adb_path = find_adb_path()?;
    
    let output = run_adb_command(&adb_path, &["-s", device_id, "shell", command])?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    Ok((stdout, stderr))
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
    let adb_path = find_adb_path()?;
    
    let mut cmd_args = vec!["-s", device_id, "shell", binary_path];
    cmd_args.extend_from_slice(args);
    
    let output = run_adb_command(&adb_path, &cmd_args)?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    Ok((stdout, stderr))
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
