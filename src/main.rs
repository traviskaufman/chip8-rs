use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use byteorder::{BigEndian, ReadBytesExt};
use clap::Parser;
use crossbeam_channel::unbounded;
use crossbeam_channel::TryRecvError;
use crossterm::event;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::{CrosstermBackend, Stylize, Terminal},
    widgets::Paragraph,
};
use std::io::stdout;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// The ROM to load into the emulator
    rom: PathBuf,
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
    // Set up core shell
    let cli = Cli::parse();
    let (killtx, killrx) = unbounded::<()>();
    // let sigkilltx = killtx.clone();
    // ctrlc::set_handler(move || sigkilltx.send(()).unwrap()).unwrap();

    // Set up memory
    let mut memory: [u8; 4096] = [0; 4096];
    fill_hex_sprites(&mut memory);

    // Set up registers
    let mut registers = Registers::new();
    let mut pc: u16 = 0x200;
    let mut sp: u8 = 0;
    // TODO: Make mut and implement stack
    let stack: [u16; 16] = [0; 16];

    // Set up timer thread
    let timer_kill_rx = killrx.clone();
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
    let delaykillrx = killrx.clone();
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
    let soundkillrx = killrx.clone();
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
    let display = Arc::new(RwLock::new([false; 64 * 32]));
    let displaykillrx = killrx.clone();
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
                        f.render_widget(
                            Paragraph::new(display_str).green().on_dark_gray(),
                            f.size(),
                        );
                    })
                    .unwrap();
            }
        })
        .unwrap();

    // Set up keyboard
    let sigkilltx = killtx.clone();
    let keyboardkillrx = killrx.clone();
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
                        sigkilltx.send(()).unwrap();
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
    // let rom_name = cli.rom.display().to_string();
    let mut rom = File::open(cli.rom).unwrap();
    rom.read(&mut memory[0x200..]).unwrap();
    // println!("Load: {} ({} bytes)", rom_name, nb);

    // Main program loop / CPU
    loop {
        match killrx.try_recv() {
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
            // Set VX to NN
            0x6000..=0x6FFF => {
                let x = (opcode & 0x0F00) >> 8;
                let nn = (opcode & 0x00FF) as u8;
                registers.v[x as usize] = nn;
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
            // 0x1NNN - Jump to address NNN
            0x1000..=0x1FFF => {
                pc = opcode & 0x0FFF;
            }
            // Return from subroutine
            0x00EE => {
                sp -= 1;
                pc = stack[sp as usize];
            }
            op => panic!("Unknown opcode: {:04X}", op),
        }
    }

    // End Program
    stdout().execute(LeaveAlternateScreen).unwrap();
    disable_raw_mode().unwrap();
    killtx.send(()).unwrap();
    timer_thread.join().unwrap();
    delay_thread.join().unwrap();
    sound_thread.join().unwrap();
    display_thread.join().unwrap();
    keyboard_thread.join().unwrap();
    println!();
}
