use std::{
    fs::File,
    io::{self, Read},
    ops::{Deref, DerefMut},
    path::Path,
};

use log::info;

/// Stores the RAM memory, can be used as proxy access to the underlying buffer.
pub struct Memory {
    buf: [u8; 4096],
}

impl Memory {
    pub fn new() -> Self {
        let mut buf = [0; 4096];
        Self::fill_hex_sprites(&mut buf);
        Self { buf }
    }

    pub fn load_rom<P: AsRef<Path>>(&mut self, rom_path: P) -> io::Result<()> {
        let rom_path = rom_path.as_ref();
        let rom_name = rom_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("(Unknown)");
        let mut rom = File::open(rom_path)?;
        let nb = rom.read(&mut self.buf[0x200..])?;
        info!("Load ROM: {} ({} bytes)", rom_name, nb);
        Ok(())
    }

    fn fill_hex_sprites(memory: &mut [u8; 4096]) {
        const HEX_SPRITES: [u8; 80] = [
            0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
            0x20, 0x60, 0x20, 0x20, 0x70, // 1
            0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
            0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
            0x90, 0x90, 0xF0, 0x10, 0x10, // 4
            0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
            0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
            0xF0, 0x10, 0x20, 0x40, 0x40, // 7
            0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
            0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
            0xF0, 0x90, 0xF0, 0x90, 0x90, // A
            0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
            0xF0, 0x80, 0x80, 0x80, 0xF0, // C
            0xE0, 0x90, 0x90, 0x90, 0xE0, // D
            0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
            0xF0, 0x80, 0xF0, 0x80, 0x80, // F
        ];

        for (i, &byte) in HEX_SPRITES.iter().enumerate() {
            memory[i] = byte;
        }
    }
}

impl Deref for Memory {
    type Target = [u8; 4096];

    fn deref(&self) -> &Self::Target {
        &self.buf
    }
}

impl DerefMut for Memory {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buf
    }
}
