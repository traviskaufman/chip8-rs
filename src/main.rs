/// TODO:
/// - Keypad test
/// - Quirks test
/// - Beep test
/// - Run an actual game
/// - Maybe implement super-chip or xo-chip
/// - Maybe implement better GUI controls and/or opcode debugging
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use byteorder::{BigEndian, ReadBytesExt};
use chip8::GameShell;
use clap::Parser;
use crossbeam_channel::unbounded;
use crossbeam_channel::TryRecvError;
use crossterm::event;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};
use std::io::stdout;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// The ROM to load into the emulator
    rom: PathBuf,
    /// Whether or not to enable the quirk for the 8XY6/8XYE instructions
    /// where shifting happens directly in the Vx register. Needed for some games.
    /// See https://tobiasvl.github.io/blog/write-a-chip-8-emulator/#logical-and-arithmetic-instructions
    #[arg(long, default_value_t = false)]
    shiftquirk: bool,
}

/// https://devernay.free.fr/hacks/chip8/C8TECH10.HTM#2.2
struct Registers {
    pub v: [u8; 16],
    pub i: u16,
    pub delay: Arc<AtomicU8>,
    pub sound: Arc<AtomicU8>,
}

impl Registers {
    pub fn new() -> Self {
        Self {
            v: [0; 16],
            i: 0,
            delay: Arc::new(AtomicU8::new(0)),
            sound: Arc::new(AtomicU8::new(0)),
        }
    }
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

/// Everything is taken from http://devernay.free.fr/hacks/chip8/C8TECH10.HTM#2.1
fn main() {
    let cli = Cli::parse();
    let gameshell = Arc::new(GameShell::new(cli.rom, cli.shiftquirk));

    // Set up memory
    let mut memory: [u8; 4096] = [0; 4096];
    fill_hex_sprites(&mut memory);

    // Set up registers
    let mut registers = Registers::new();
    let mut pc: u16 = 0x200;
    let mut sp: u8 = 0;
    // TODO: Make mut and implement stack
    let mut stack: [u16; 16] = [0; 16];

    // Set up timer thread
    let timer_kill_rx = gameshell.killrx();
    let (timertx, timerrx) = unbounded();
    let timer_thread = thread::Builder::new()
        .name("timer".to_string())
        .spawn(move || {
            loop {
                match timer_kill_rx.try_recv() {
                    Err(TryRecvError::Empty) => {}
                    _ => break,
                }
                // 60Hz
                std::thread::sleep(std::time::Duration::from_millis(16));
                timertx.send(std::time::Instant::now()).unwrap();
            }
        })
        .unwrap();

    // Set up delay+sound threads
    let delay = registers.delay.clone();
    let delaykillrx = gameshell.killrx();
    let delaytimerrx = timerrx.clone();
    let delay_thread = thread::Builder::new()
        .name("timer".to_string())
        .spawn(move || loop {
            match delaykillrx.try_recv() {
                Err(TryRecvError::Empty) => {}
                _ => break,
            }
            if let Err(_) = delaytimerrx.recv() {
                break;
            }

            let vdelay = delay.load(Ordering::Acquire);
            if vdelay > 0 {
                delay.store(vdelay - 1, Ordering::Release);
            }
        })
        .unwrap();

    let sound = registers.sound.clone();
    let soundkillrx = gameshell.killrx();
    let soundtimerrx = timerrx.clone();
    let sound_thread = thread::Builder::new()
        .name("sound".to_string())
        .spawn(move || loop {
            match soundkillrx.try_recv() {
                Err(TryRecvError::Empty) => {}
                _ => break,
            }
            if let Err(_) = soundtimerrx.recv() {
                break;
            }

            let vsound = sound.load(Ordering::Acquire);
            if vsound > 0 {
                // TODO: Make actual sound
                sound.store(vsound - 1, Ordering::Release);
            }
        })
        .unwrap();

    // Set up display
    let rom_title = gameshell.rom_title();
    let display = Arc::new(RwLock::new([false; 64 * 32]));
    let displaykillrx = gameshell.killrx();
    let displaytimerrx = timerrx.clone();
    let render_display = display.clone();
    stdout().execute(EnterAlternateScreen).unwrap();
    enable_raw_mode().unwrap();
    let display_thread = thread::Builder::new()
        .name("display".to_string())
        .spawn(move || {
            let backend = CrosstermBackend::new(stdout());
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.clear().unwrap();

            loop {
                match displaykillrx.try_recv() {
                    Err(TryRecvError::Empty) => {}
                    _ => break,
                }
                if let Err(_) = displaytimerrx.recv() {
                    break;
                }

                let mut display_str = String::new();
                let display = render_display.read().unwrap();
                for (i, &pixel) in display.iter().enumerate() {
                    display_str.push(if pixel { '█' } else { ' ' });
                    if i % 64 == 63 {
                        display_str.push('\n');
                    }
                }
                terminal
                    .draw(|f| {
                        f.render_widget(Block::new().on_black(), f.size());

                        let layout = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints(vec![
                                Constraint::Length(3),
                                Constraint::Length(32),
                                Constraint::Fill(1),
                            ])
                            .split(f.size());

                        let title = layout[0];
                        f.render_widget(
                            Paragraph::new(format!("[Chip8-RS] {}", rom_title))
                                .white()
                                .centered()
                                .block(Block::bordered()),
                            title,
                        );

                        let emu_layout = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints(vec![
                                Constraint::Fill(1),
                                Constraint::Length(64),
                                Constraint::Fill(1),
                            ])
                            .split(layout[1]);
                        let emu = emu_layout[1];
                        f.render_widget(Paragraph::new(display_str).light_blue().on_black(), emu);
                    })
                    .unwrap();
            }
        })
        .unwrap();

    // Set up keyboard
    let keyboard_gameshell = gameshell.clone();
    let keyboardkillrx = gameshell.killrx();
    let keyboard_thread = thread::Builder::new()
        .name("keyboard".to_string())
        .spawn(move || loop {
            match keyboardkillrx.try_recv() {
                Err(TryRecvError::Empty) => {}
                _ => break,
            }
            // If read ctrl+c from crossterm, send kill signal
            if let Ok(evt) = event::read() {
                match evt {
                    event::Event::Key(event::KeyEvent {
                        code: event::KeyCode::Char('c'),
                        modifiers: event::KeyModifiers::CONTROL,
                        ..
                    }) => {
                        keyboard_gameshell.kill();
                        break;
                    }
                    // TODO: Get other keyboard inputs working
                    _ => {}
                }
            }
        })
        .unwrap();

    // Load ROM
    // TODO: Logging
    let mut rom = File::open(gameshell.rom_path()).unwrap();
    rom.read(&mut memory[0x200..]).unwrap();
    // println!("Load: {} ({} bytes)", rom_name, nb);

    // Main program loop / CPU
    loop {
        match gameshell.killrx().try_recv() {
            Err(TryRecvError::Empty) => {}
            _ => break,
        }

        let opcode = Cursor::new(&memory[pc as usize..])
            .read_u16::<BigEndian>()
            .unwrap();
        // println!("{:04X}: {:04X}", pc, opcode);
        pc += 2;

        match opcode {
            // Clear the screen
            0x00E0 => {
                let mut display = display.write().unwrap();
                for pixel in display.iter_mut() {
                    *pixel = false;
                }
            }
            // Return from subroutine
            0x00EE => {
                sp -= 1;
                pc = stack[sp as usize];
            }
            // 0x1NNN - Jump to address NNN
            0x1000..=0x1FFF => {
                pc = opcode & 0x0FFF;
            }
            // 2nnn - CALL addr
            // Call subroutine at nnn.
            // The interpreter increments the stack pointer, then puts the current PC on the top of the stack. The PC is then set to nnn.
            0x2000..=0x2FFF => {
                stack[sp as usize] = pc;
                sp += 1;
                pc = opcode & 0x0FFF;
            }
            // 3xkk - SE Vx, byte
            // Skip next instruction if Vx = kk.
            // The interpreter compares register Vx to kk, and if they are equal, increments the program counter by 2.
            0x3000..=0x3FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let kk = (opcode & 0x00FF) as u8;
                if registers.v[x as usize] == kk {
                    pc += 2;
                }
            }
            //4xkk - SNE Vx, byte
            // Skip next instruction if Vx != kk.
            // The interpreter compares register Vx to kk, and if they are not equal, increments the program counter by 2.
            0x4000..=0x4FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let kk = (opcode & 0x00FF) as u8;
                if registers.v[x as usize] != kk {
                    pc += 2;
                }
            }
            // 5xy0 - SE Vx, Vy
            // Skip next instruction if Vx = Vy.
            // The interpreter compares register Vx to register Vy, and if they are equal, increments the program counter by 2.
            0x5000..=0x5FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let y = (opcode & 0x00F0) >> 4;
                if x == y {
                    pc += 2;
                }
            }
            // Set VX to NN
            0x6000..=0x6FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let nn = (opcode & 0x00FF) as u8;
                registers.v[x as usize] = nn;
            }
            0x7000..=0x7FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let kk = (opcode & 0x00FF) as u8;
                registers.v[x as usize] = registers.v[x as usize].wrapping_add(kk);
            }
            // 8xy0 - LD Vx, Vy
            // Set Vx = Vy.
            // Stores the value of register Vy in register Vx.
            0x8000..=0x8FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let y = (opcode & 0x00F0) >> 4;
                let op = opcode & 0x000F;
                match op {
                    // LD Vx, Vy
                    0x0 => {
                        registers.v[x as usize] = registers.v[y as usize];
                    }
                    // OR Vx, Vy
                    0x1 => {
                        registers.v[x as usize] |= registers.v[y as usize];
                    }
                    // AND Vx, Vy
                    0x2 => {
                        registers.v[x as usize] &= registers.v[y as usize];
                    }
                    // XOR Vx, Vy
                    0x3 => {
                        registers.v[x as usize] ^= registers.v[y as usize];
                    }
                    // ADD Vx, Vy
                    0x4 => {
                        let (res, overflow) =
                            registers.v[x as usize].overflowing_add(registers.v[y as usize]);
                        registers.v[x as usize] = res;
                        registers.v[0xF] = overflow as u8;
                    }
                    // SUB Vx, Vy
                    0x5 => {
                        let (res, overflow) =
                            registers.v[x as usize].overflowing_sub(registers.v[y as usize]);
                        registers.v[x as usize] = res;
                        // NOT borrow
                        registers.v[0xF] = !overflow as u8;
                    }
                    // SHR Vx {, Vy} ... todo will maybe have to revisit this
                    0x6 => {
                        if !cli.shiftquirk {
                            registers.v[x as usize] = registers.v[y as usize];
                        }
                        let flag = registers.v[x as usize] & 0x1;
                        registers.v[x as usize] >>= 1;
                        registers.v[0xF] = flag;
                    }
                    // SUBN Vx, Vy
                    0x7 => {
                        let (res, overflow) =
                            registers.v[y as usize].overflowing_sub(registers.v[x as usize]);
                        registers.v[x as usize] = res;
                        registers.v[0xF] = !overflow as u8;
                    }
                    // SHL Vx {, Vy}
                    0xE => {
                        if !cli.shiftquirk {
                            registers.v[x as usize] = registers.v[y as usize];
                        }
                        let flag = (registers.v[x as usize] & 0x80) >> 7;
                        registers.v[x as usize] <<= 1;
                        registers.v[0xF] = flag;
                    }
                    _ => panic!("Unknown opcode instruction {:04X}", opcode),
                }
            }
            // 9xy0 - SNE Vx, Vy
            // Skip next instruction if Vx != Vy.
            // The values of Vx and Vy are compared, and if they are not equal, the program counter is increased by 2.
            0x9000..=0x9FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let y = (opcode & 0x00F0) >> 4;
                if registers.v[x as usize] != registers.v[y as usize] {
                    pc += 2;
                }
            }
            // Set I to NNN
            0xA000..=0xAFFF => {
                registers.i = opcode & 0x0FFF;
            }
            // Dxyn - Display n-byte sprite starting at memory location I at (Vx, Vy), set VF = collision.
            0xD000..=0xDFFF => {
                let x = (opcode & 0x0F00) >> 8;
                let y = (opcode & 0x00F0) >> 4;
                let n = opcode & 0x000F;
                let vx = registers.v[x as usize] as usize;
                let vy = registers.v[y as usize] as usize;
                let mut collision = false;

                let mut display = display.write().unwrap();
                for byteidx in 0..n {
                    let byte = memory[(registers.i + byteidx) as usize];
                    for bitidx in 0..8 {
                        let bit = (byte >> (7 - bitidx)) & 1;
                        // Wrap around the screen if needed
                        let idx = (vx + bitidx as usize) % 64 + ((vy + byteidx as usize) % 32) * 64;
                        if display[idx] && bit == 1 {
                            collision = true;
                        }
                        display[idx] ^= bit == 1;
                    }
                }
                registers.v[0xF] = collision as u8;
            }
            0xF000..=0xFFFF => {
                let x = (opcode & 0x0F00) >> 8;
                let op = opcode & 0x00FF;
                match op {
                    // Fx07 - LD Vx, DT
                    // Set Vx = delay timer value.
                    // The value of DT is placed into Vx.
                    0x07 => {
                        registers.v[x as usize] = registers.delay.load(Ordering::Acquire);
                    }
                    // Fx0A - LD Vx, K
                    // Wait for a key press, store the value of the key in Vx.
                    // All execution stops until a key is pressed, then the value of that key is stored in Vx.
                    0x0A => {
                        // TODO: Keypress
                    }
                    // Fx15 - LD DT, Vx
                    // Set delay timer = Vx.
                    // DT is set equal to the value of Vx.
                    0x15 => {
                        registers
                            .delay
                            .store(registers.v[x as usize], Ordering::Relaxed);
                    }
                    // Fx18 - LD ST, Vx
                    // Set sound timer = Vx.
                    // ST is set equal to the value of Vx.
                    0x18 => {
                        registers
                            .sound
                            .store(registers.v[x as usize], Ordering::Relaxed);
                    }
                    // Fx1E - ADD I, Vx
                    // Set I = I + Vx.
                    // The values of I and Vx are added, and the results are stored in I.
                    0x1E => {
                        registers.i += registers.v[x as usize] as u16;
                    }
                    // Fx29 - LD F, Vx
                    // Set I = location of sprite for digit Vx.
                    // The value of I is set to the location for the hexadecimal sprite corresponding to the value of Vx.
                    0x29 => {
                        // Sprites are indexed from 0x0000 in memory
                        registers.i = registers.v[x as usize] as u16 * 5;
                    }
                    // Fx33 - LD B, Vx
                    // Store BCD representation of Vx in memory locations I, I+1, and I+2.
                    // The interpreter takes the decimal value of Vx, and places the hundreds digit in memory at location in I,
                    // the tens digit at location I+1, and the ones digit at location I+2.
                    0x33 => {
                        let vx = registers.v[x as usize];
                        memory[registers.i as usize] = vx / 100;
                        memory[(registers.i + 1) as usize] = (vx / 10) % 10;
                        memory[(registers.i + 2) as usize] = vx % 10;
                    }
                    // Fx55 - LD [I], Vx
                    // Store registers V0 through Vx in memory starting at location I.
                    // The interpreter copies the values of registers V0 through Vx into memory, starting at the address in I.
                    0x55 => {
                        for i in 0..=x {
                            memory[(registers.i + i) as usize] = registers.v[i as usize];
                        }
                    }
                    // Fx65 - LD Vx, [I]
                    // Read registers V0 through Vx from memory starting at location I.
                    // The interpreter reads values from memory starting at location I into registers V0 through Vx.
                    0x65 => {
                        for i in 0..=x {
                            registers.v[i as usize] = memory[(registers.i + i) as usize];
                        }
                    }
                    op => panic!("Unknown opcode instruction {:04X}", op),
                }
            }
            op => panic!("Unknown opcode: {:04X}", op),
        }
    }

    // End Program
    stdout().execute(LeaveAlternateScreen).unwrap();
    disable_raw_mode().unwrap();
    gameshell.kill();
    timer_thread.join().unwrap();
    delay_thread.join().unwrap();
    sound_thread.join().unwrap();
    display_thread.join().unwrap();
    keyboard_thread.join().unwrap();
    println!();
}
