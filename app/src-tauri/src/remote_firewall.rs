//! Explicit, user-initiated Windows firewall setup. Never elevates at app startup.

#[tauri::command]
pub async fn remote_allow_firewall() -> Result<(), String> {
    #[cfg(not(windows))]
    return Err("This action is only available on Windows.".into());
    #[cfg(windows)]
    tauri::async_runtime::spawn_blocking(configure).await.map_err(|e| e.to_string())?
}

#[cfg(windows)]
fn configure() -> Result<(), String> {
    use base64::Engine;
    use std::os::windows::process::CommandExt;
    // The rule follows the executable, including when the user changes the listening port.
    // Restrict it to Private profiles and local subnet peers.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let program = exe.to_string_lossy().replace('\'', "''");
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$program = '{program}'
foreach ($protocol in @('TCP', 'UDP')) {{
  $rule = 'oah-phone-' + $protocol
  Get-NetFirewallRule -Name $rule -ErrorAction SilentlyContinue | Remove-NetFirewallRule
  if ($protocol -eq 'UDP') {{
    New-NetFirewallRule -Name $rule -DisplayName 'oah nearby discovery' -Direction Inbound -Action Allow -Program $program -Protocol UDP -LocalPort 7788 -Profile Private -RemoteAddress LocalSubnet | Out-Null
  }} else {{
    New-NetFirewallRule -Name $rule -DisplayName 'oah phone connections' -Direction Inbound -Action Allow -Program $program -Protocol TCP -Profile Private -RemoteAddress LocalSubnet | Out-Null
  }}
}}
"#
    );
    let encode =
        |s: &str| base64::engine::general_purpose::STANDARD.encode(s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>());
    // Encode both scripts: paths and arguments never pass through shell string interpolation.
    let elevated = encode(&script);
    let launcher = format!(
        r#"$ErrorActionPreference = 'Stop'; try {{ $p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList '-NoProfile -NonInteractive -EncodedCommand {elevated}'; exit $p.ExitCode }} catch {{ exit 1 }}"#
    );
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encode(&launcher)])
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| format!("Could not start Windows firewall setup: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Firewall setup was cancelled or failed. Allow the Windows administrator prompt, and check that your trusted home network is Private. Existing block rules or managed firewall policies may still prevent connections.".into())
    }
}
