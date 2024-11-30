use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender, TryRecvError};

mod cpu;
pub mod logger;
mod memory;
pub use cpu::CPU;
pub use memory::Memory;

#[derive(Clone)]
pub struct KillSignal {
    tx: Sender<()>,
    rx: Receiver<()>,
}

impl KillSignal {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { tx, rx }
    }

    /// NOTE: A kill signal is a one-shot signal. Once it's received, it's gone.
    /// Thus, it is not guaranteed that if `received()` is true, it will be true subsequently.
    pub fn received(&self) -> bool {
        match self.rx.try_recv() {
            Err(TryRecvError::Empty) => false,
            _ => true,
        }
    }

    pub fn send(&self) {
        self.tx.send(()).unwrap();
    }
}

pub struct GameShell {
    pub rom: PathBuf,
    pub shiftquirk: bool,
    killsignal_internal: KillSignal,
}

impl GameShell {
    pub fn new(rom: PathBuf, shiftquirk: bool) -> Self {
        Self {
            rom,
            shiftquirk,
            killsignal_internal: KillSignal::new(),
        }
    }

    pub fn rom_path(&self) -> &PathBuf {
        &self.rom
    }

    pub fn print_rom_title(&self) -> String {
        self.rom.display().to_string()
    }

    pub fn clone_killsignal(&self) -> KillSignal {
        self.killsignal_internal.clone()
    }
}
