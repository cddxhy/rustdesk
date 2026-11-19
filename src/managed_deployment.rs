use std::{fs, io::ErrorKind, path::Path, time::Duration};

use hbb_common::{
    anyhow::{anyhow, Context},
    base64::{
        engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
        Engine as _,
    },
    config, log,
    sodiumoxide::{
        crypto::{box_, sealedbox},
        randombytes,
    },
    tokio, ResultType,
};
use serde_derive::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

const API_URL: Option<&str> = option_env!("MANAGEMENT_API_URL");
const STARTUP_ROUNDS: u8 = 3;
const STARTUP_DELAY: Duration = Duration::from_secs(30);
const NORMAL_DELAY: Duration = Duration::from_secs(60 * 60);

pub(crate) fn enabled() -> bool {
    option_env!("RUSTDESK_MANAGED_BUILD") == Some("1")
}

pub(crate) fn uses_installed_service() -> bool {
    enabled() && crate::platform::is_installed()
}

pub(crate) fn on_client_start() {
    if crate::platform::is_self_service_running() {
        std::thread::spawn(|| {
            if let Err(error) = request_startup_sync() {
                log::debug!("Managed service did not receive client startup: {error}");
            }
        });
    } else if let Err(error) = crate::platform::windows::start_managed_service() {
        log::warn!("Failed to start the managed service: {error}");
    } else if !crate::check_process("--tray", true) {
        hbb_common::allow_err!(crate::run_me(vec!["--tray"]));
    }
}

#[tokio::main(flavor = "current_thread")]
async fn request_startup_sync() -> ResultType<()> {
    let mut service = crate::ipc::connect_service(1_000).await?;
    service.send(&crate::ipc::Data::ManagedClientStarted).await
}

pub(crate) fn relay_server() -> &'static str {
    option_env!("RUSTDESK_RELAY_SERVER").unwrap_or_default()
}

pub(crate) fn install_policy() {
    use base::config::keys;

    config::OVERWRITE_SETTINGS.write().unwrap().extend(
        [
            (
                "custom-rendezvous-server",
                option_env!("RUSTDESK_ID_SERVER").unwrap_or_default(),
            ),
            ("relay-server", relay_server()),
            (
                "key",
                option_env!("RUSTDESK_SERVER_PUBLIC_KEY").unwrap_or_default(),
            ),
            ("allow-auto-update", "N"),
            (keys::OPTION_ALLOW_REMOTE_CONFIG_MODIFICATION, "Y"),
            (keys::OPTION_ALLOW_WEBSOCKET, "N"),
            (keys::OPTION_ALLOW_INSECURE_TLS_FALLBACK, "N"),
            (keys::OPTION_DISABLE_UDP, "N"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned())),
    );
    config::OVERWRITE_LOCAL_SETTINGS
        .write()
        .unwrap()
        .insert("enable-check-update".to_owned(), "N".to_owned());
    config::BUILTIN_SETTINGS.write().unwrap().extend(
        [
            ("managed-deployment-api", API_URL.unwrap_or_default()),
            (keys::OPTION_HIDE_NETWORK_SETTINGS, "Y"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned())),
    );
    if crate::platform::windows::is_virtualbox_guest() {
        config::DEFAULT_SETTINGS.write().unwrap().insert(
            keys::OPTION_ENABLE_DIRECTX_CAPTURE.to_owned(),
            "N".to_owned(),
        );
        config::DEFAULT_LOCAL_SETTINGS
            .write()
            .unwrap()
            .insert(keys::OPTION_ALLOW_D3D_RENDER.to_owned(), "N".to_owned());
    }
}

#[derive(Deserialize, Serialize)]
struct Device {
    id: String,
    bearer: String,
    enrolled: bool,
    version: u32,
    rustdesk_id: String,
    pending: Option<Pending>,
}

#[derive(Deserialize, Serialize)]
struct Pending {
    version: u32,
    rustdesk_id: String,
    password: String,
    operation_id: String,
}

#[derive(Deserialize)]
struct PendingActions {
    minimum_version: u32,
}

pub(crate) async fn run(opened: tokio::sync::watch::Receiver<()>) {
    if let Err(error) = poll(opened).await {
        log::warn!("Managed device worker stopped: {error}");
    }
}

async fn poll(mut opened: tokio::sync::watch::Receiver<()>) -> ResultType<()> {
    let directory = crate::platform::windows::managed_state_directory()?;
    let api = API_URL
        .ok_or_else(|| anyhow!("Management API URL is missing"))?
        .trim_end_matches('/');
    let public_key =
        STANDARD.decode(option_env!("MANAGEMENT_API_ENCRYPTION_PUBLIC_KEY").unwrap_or_default())?;
    let public_key = box_::PublicKey::from_slice(&public_key)
        .ok_or_else(|| anyhow!("Invalid management API encryption key"))?;
    let http = reqwest::Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut startup_rounds = STARTUP_ROUNDS;
    loop {
        if let Err(error) = sync(&directory, &http, api, &public_key).await {
            log::warn!("Managed device sync failed: {error}");
        }
        if !wait_for_poll(&mut opened, &mut startup_rounds).await {
            return Ok(());
        }
    }
}

async fn wait_for_poll(
    opened: &mut tokio::sync::watch::Receiver<()>,
    startup_rounds: &mut u8,
) -> bool {
    if *startup_rounds > 0 {
        *startup_rounds -= 1;
        // Coalesce client launches during the startup burst.
        opened.borrow_and_update();
        if *startup_rounds > 0 {
            tokio::time::sleep(STARTUP_DELAY).await;
            return true;
        }
    }
    tokio::select! {
        changed = opened.changed() => {
            *startup_rounds = STARTUP_ROUNDS;
            changed.is_ok()
        }
        _ = tokio::time::sleep(NORMAL_DELAY) => true,
    }
}

async fn wait_for_rustdesk_id() -> String {
    loop {
        match crate::ipc::get_config_async("id", 1_000).await {
            Ok(Some(id)) if !id.is_empty() => return id,
            result => hbb_common::throttled_log!(
                30,
                debug,
                "Waiting for the trusted RustDesk server before management sync: {:?}",
                result.err()
            ),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn sync(
    directory: &Path,
    http: &reqwest::Client,
    api: &str,
    public_key: &box_::PublicKey,
) -> ResultType<()> {
    let state_path = directory.join("device.json");
    let mut device: Device = match fs::read(&state_path) {
        Ok(value) => serde_json::from_slice(&value).context("Invalid device state")?,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let device = Device {
                id: Uuid::new_v4().to_string(),
                bearer: random_secret(),
                enrolled: false,
                version: 0,
                rustdesk_id: String::new(),
                pending: None,
            };
            save(&state_path, &device)?;
            device
        }
        Err(error) => return Err(error.into()),
    };
    let rustdesk_id = wait_for_rustdesk_id().await;

    if !device.enrolled {
        checked(http.post(format!("{api}/v1/devices/enroll")).json(&json!({
            "device_id": device.id,
            "device_token": device.bearer,
        })))
        .await?;
        device.enrolled = true;
        save(&state_path, &device)?;
    }

    let base = format!("{api}/v1/devices/{}", device.id);
    let actions: PendingActions = checked(
        http.get(format!("{base}/pending-actions"))
            .bearer_auth(&device.bearer),
    )
    .await?
    .json()
    .await
    .context("Invalid pending-actions response")?;

    let needs_new_version = match &device.pending {
        Some(pending) => {
            pending.version < actions.minimum_version || pending.rustdesk_id != rustdesk_id
        }
        None => {
            device.version == 0
                || actions.minimum_version > device.version
                || device.rustdesk_id != rustdesk_id
        }
    };
    if needs_new_version {
        let last_version = device
            .pending
            .as_ref()
            .map_or(device.version, |pending| pending.version);
        let next_version = last_version
            .checked_add(1)
            .ok_or_else(|| anyhow!("Credential version exhausted"))?;
        device.pending = Some(Pending {
            version: actions.minimum_version.max(next_version),
            rustdesk_id,
            password: random_secret(),
            operation_id: Uuid::new_v4().to_string(),
        });
        save(&state_path, &device)?;
    }

    if let Some(pending) = &device.pending {
        let payload = serde_json::to_vec(&json!({
            "device_id": device.id,
            "rustdesk_id": pending.rustdesk_id,
            "password": pending.password,
            "version": pending.version,
            "operation_id": pending.operation_id,
        }))?;
        let ciphertext = STANDARD.encode(sealedbox::seal(&payload, public_key));
        checked(
            http.post(format!("{base}/credentials"))
                .bearer_auth(&device.bearer)
                .json(&json!({
                    "key_id": option_env!("MANAGEMENT_API_ENCRYPTION_KEY_ID").unwrap_or_default(),
                    "ciphertext": ciphertext,
                })),
        )
        .await?;
        if !crate::ipc::set_permanent_password_with_ack_async(pending.password.clone()).await? {
            return Err(anyhow!("RustDesk service rejected the managed password"));
        }
        checked(
            http.post(format!("{base}/credentials/{}/activate", pending.version))
                .bearer_auth(&device.bearer)
                .json(&json!({"operation_id": pending.operation_id})),
        )
        .await?;
        device.version = pending.version;
        device.rustdesk_id = pending.rustdesk_id.clone();
        device.pending = None;
        save(&state_path, &device)?;
    }

    Ok(())
}

async fn checked(request: reqwest::RequestBuilder) -> ResultType<reqwest::Response> {
    let response = request
        .send()
        .await
        .map_err(|_| anyhow!("Management API request failed"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Management API returned HTTP {}",
            response.status().as_u16()
        ));
    }
    Ok(response)
}

fn random_secret() -> String {
    URL_SAFE_NO_PAD.encode(randombytes::randombytes(32))
}

fn save(path: &Path, device: &Device) -> ResultType<()> {
    use std::io::Write;
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec(device)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{sync::watch, time::Instant};

    #[tokio::test(start_paused = true)]
    async fn three_startup_rounds_then_hourly() {
        let (opened, mut receiver) = watch::channel(());
        let mut startup_rounds = STARTUP_ROUNDS;
        let start = Instant::now();
        assert!(wait_for_poll(&mut receiver, &mut startup_rounds).await);
        assert_eq!(start.elapsed(), STARTUP_DELAY);

        opened.send_replace(());
        assert!(wait_for_poll(&mut receiver, &mut startup_rounds).await);
        assert_eq!(start.elapsed(), STARTUP_DELAY * 2);
        assert!(wait_for_poll(&mut receiver, &mut startup_rounds).await);
        assert_eq!(start.elapsed(), STARTUP_DELAY * 2 + NORMAL_DELAY);
        assert_eq!(startup_rounds, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn opening_the_client_interrupts_the_hourly_wait() {
        let (opened, mut receiver) = watch::channel(());
        let mut startup_rounds = 0;
        let start = Instant::now();
        opened.send_replace(());
        assert!(wait_for_poll(&mut receiver, &mut startup_rounds).await);
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert_eq!(startup_rounds, STARTUP_ROUNDS);
    }
}
