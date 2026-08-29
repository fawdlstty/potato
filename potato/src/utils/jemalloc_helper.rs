#![cfg(all(feature = "jemalloc", not(target_os = "windows")))]

use anyhow::anyhow;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use tikv_jemalloc_ctl::{raw, Access, AsName};
use tokio::fs;

pub use tikv_jemallocator::Jemalloc as JemallocAllocator;

static INIT_JEMALLOC: AtomicBool = AtomicBool::new(true);

pub fn init_jemalloc() -> anyhow::Result<()> {
    if !INIT_JEMALLOC.load(Ordering::SeqCst) {
        return Ok(());
    }

    if let Ok(conf) = std::env::var("MALLOC_CONF") {
        if conf.contains("prof:true") {
            const PROF_ACTIVE: &[u8] = b"prof.active\0";
            let name = PROF_ACTIVE.name();
            match name.write(true) {
                Ok(()) => {
                    INIT_JEMALLOC.store(false, Ordering::SeqCst);
                    return Ok(());
                }
                Err(err) => Err(anyhow!("{err}"))?,
            }
        }
    }
    Err(anyhow::anyhow!(
        "run `MALLOC_CONF=prof:true {}` for enable jemalloc, or disable jemalloc features",
        env!("CARGO_PKG_NAME")
    ))
}

pub async fn dump_jemalloc_profile() -> anyhow::Result<Vec<u8>> {
    let prof_dir = std::env::temp_dir().join(format!(
        "{}-prof-{}-{}",
        env!("CARGO_PKG_NAME"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    #[cfg(unix)]
    fs::DirBuilder::new().mode(0o700).create(&prof_dir).await?;
    #[cfg(not(unix))]
    fs::create_dir(&prof_dir).await?;

    let prof_file = prof_dir.join("profile.dump");
    let prof_name = CString::new(prof_file.as_os_str().as_encoded_bytes())?;
    let prof_name = Box::leak(prof_name.into_bytes().into_boxed_slice());
    let mut mib = [0usize; 1];
    raw::name_to_mib(b"prof.dump\0", &mut mib).map_err(|err| anyhow!("{err}"))?;
    raw::write_str_mib(&mib, prof_name).map_err(|err| anyhow!("{err}"))?;

    let cur_path = {
        let cur_path = std::env::current_exe()?;
        cur_path
            .to_str()
            .ok_or(anyhow!("path convert failed"))?
            .to_string()
    };

    let output = tokio::process::Command::new("jeprof")
        .args(["--show_bytes", "--pdf", &cur_path])
        .arg(&prof_file)
        .output()
        .await?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(anyhow!("jeprof failed: {message}"))?;
    }

    let buf = output.stdout;
    let _ = fs::remove_dir_all(&prof_dir).await;
    Ok(buf)
}
