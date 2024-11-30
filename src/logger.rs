use anyhow::Result;
use std::{fs, io, path::Path};

pub fn init<P: AsRef<Path>>(log_file: P) -> Result<()> {
    let log_file = log_file.as_ref();
    rm_rf(log_file)?;
    simple_logging::log_to_file(log_file, log::LevelFilter::Info)?;
    Ok(())
}

fn rm_rf<P: AsRef<Path>>(path: P) -> Result<(), io::Error> {
    let path = path.as_ref();
    match fs::remove_file(path) {
        Ok(_) => Ok(()),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
