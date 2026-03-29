#[cfg(not(target_os = "windows"))]
use crate::utils::process::ProgramRunner;
use anyhow::anyhow;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_os = "windows"))]
use tikv_jemalloc_ctl::*;
#[cfg(not(target_os = "windows"))]
use tokio::fs::{self, File};
#[cfg(not(target_os = "windows"))]
use tokio::io::AsyncReadExt;

#[cfg(all(feature = "jemalloc", not(target_os = "windows")))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

static INIT_JEMALLOC: AtomicBool = AtomicBool::new(true);

#[cfg(not(target_os = "windows"))]
pub fn init_jemalloc() -> anyhow::Result<()> {
    if !INIT_JEMALLOC.load(Ordering::SeqCst) {
        return Ok(());
    }

    if let Ok(conf) = std::env::var("MALLOC_CONF") {
        if conf.contains("prof:true") {
            const PROF_ACTIVE: &'static [u8] = b"prof.active\0";
            let name = PROF_ACTIVE.name();
            match name.write(true) {
                Ok(()) => {
                    INIT_JEMALLOC.store(false, Ordering::SeqCst);
                    return Ok(());
                }
                Err(err) => Err(anyhow!("{}", err.to_string()))?,
            }
        }
    }
    Err(anyhow::anyhow!(
        "run `MALLOC_CONF=prof:true {}` for enable jemalloc, or disable jemalloc features",
        env!("CARGO_PKG_NAME")
    ))
}

#[cfg(target_os = "windows")]
pub fn init_jemalloc() -> anyhow::Result<()> {
    if !INIT_JEMALLOC.load(Ordering::SeqCst) {
        return Err(anyhow!(
            "jemalloc feature is not available on Windows; disable jemalloc or build on a non-Windows target"
        ));
    }
    Err(anyhow!(
        "jemalloc feature is not available on Windows; disable jemalloc or build on a non-Windows target"
    ))
}

#[cfg(not(target_os = "windows"))]
pub async fn dump_jemalloc_profile() -> anyhow::Result<Vec<u8>> {
    const PROF_DUMP: &'static [u8] = b"prof.dump\0";
    let prof_file = format!("/tmp/prof_{}.dump", env!("CARGO_PKG_NAME"));
    let pdf_file = format!("{prof_file}.pdf");
    _ = fs::remove_file(&prof_file).await;
    _ = fs::remove_file(&pdf_file).await;
    let prof_name = format!("{prof_file}\0").into_boxed_str();
    let prof_name_ptr: &'static [u8] = unsafe { std::mem::transmute(prof_name) };
    let name = PROF_DUMP.name();
    if let Err(err) = name.write(prof_name_ptr) {
        Err(anyhow!("{}", err.to_string()))?;
    }
    let cur_path = {
        let cur_path = std::env::current_exe()?;
        cur_path
            .to_str()
            .ok_or(anyhow!("path convert failed"))?
            .to_string()
    };
    let cmd_str = format!("jeprof --show_bytes --pdf {cur_path} {prof_file} > {pdf_file}");
    let out = ProgramRunner::run_until_exit(&cmd_str).await;
    let buf = {
        let mut buf = Vec::with_capacity(4096);
        let mut f = File::open(&pdf_file).await?;
        f.read_to_end(&mut buf).await?;
        buf
    };
    if buf.is_empty() {
        let out = out?;
        Err(anyhow!("{out}"))?;
    }
    _ = fs::remove_file(&prof_file).await;
    _ = fs::remove_file(&pdf_file).await;
    Ok(buf)
}

#[cfg(target_os = "windows")]
pub async fn dump_jemalloc_profile() -> anyhow::Result<Vec<u8>> {
    Err(anyhow!("jemalloc profile dump is not available on Windows"))
}
