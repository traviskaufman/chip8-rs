use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender};

pub struct GameShell {
    pub rom: PathBuf,
    pub shiftquirk: bool,
    killtx: Sender<()>,
    killrx: Receiver<()>,
}

impl GameShell {
    pub fn new(rom: PathBuf, shiftquirk: bool) -> Self {
        let (killtx, killrx) = crossbeam_channel::unbounded();
        Self {
            rom,
            shiftquirk,
            killtx,
            killrx,
        }
    }

    pub fn rom_path(&self) -> &PathBuf {
        &self.rom
    }

    pub fn rom_title(&self) -> String {
        self.rom.display().to_string()
    }

    pub fn killrx(&self) -> Receiver<()> {
        self.killrx.clone()
    }

    pub fn kill(&self) {
        self.killtx.send(()).unwrap();
    }
}
