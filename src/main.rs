/// TODO:
/// - Keypad test
/// - Quirks test
/// - Beep test
/// - Run an actual game
/// - Maybe implement super-chip or xo-chip
/// - Maybe implement better GUI controls and/or opcode debugging
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use byteorder::{BigEndian, ReadBytesExt};
use chip8::{logger, GameShell, Memory};
use clap::Parser;
use crossterm::event;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use log::info;
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

/// Everything is taken from http://devernay.free.fr/hacks/chip8/C8TECH10.HTM#2.1
fn main() {
    logger::init("chip8.log").unwrap();

    let cli = Cli::parse();
    let gameshell = GameShell::new(cli.rom, cli.shiftquirk);

    // Set up memory
    let mut memory = Memory::new();

    // Set up registers
    let mut registers = Registers::new();
    let mut pc: u16 = 0x200;
    let mut sp: u8 = 0;
    // TODO: Make mut and implement stack
    let mut stack: [u16; 16] = [0; 16];

    // Set up display
    let rom_title = gameshell.print_rom_title();
    let display = Arc::new(RwLock::new([false; 64 * 32]));

    memory.load_rom(gameshell.rom_path()).unwrap();

    // Main program loop / CPU
    let mainkill = gameshell.clone_killsignal();
    let mut previous = std::time::Instant::now();
    let mut lag = std::time::Duration::from_millis(0);
    /// 60Hz
    const FRAMERATE: Duration = std::time::Duration::from_millis(16);

    stdout().execute(EnterAlternateScreen).unwrap();
    enable_raw_mode().unwrap();

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.clear().unwrap();

    loop {
        let current = std::time::Instant::now();
        let elapsed = current - previous;
        previous = current;
        lag += elapsed;

        if mainkill.received() {
            break;
        }

        // If read ctrl+c from crossterm, send kill signal
        match event::poll(std::time::Duration::from_millis(0)) {
            Ok(true) => {
                if let Ok(evt) = event::read() {
                    match evt {
                        event::Event::Key(event::KeyEvent {
                            code: event::KeyCode::Char('c'),
                            modifiers: event::KeyModifiers::CONTROL,
                            ..
                        }) => {
                            break;
                        }
                        // TODO: Get other keyboard inputs working
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        while lag >= FRAMERATE {
            update(
                &mut memory,
                &mut pc,
                &display,
                &mut sp,
                &mut stack,
                &mut registers,
                cli.shiftquirk,
            );
            lag -= FRAMERATE
        }

        let mut display_str = String::new();
        let display = display.read().unwrap();
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

        std::thread::sleep(FRAMERATE - lag);
    }

    // end program
    stdout().execute(LeaveAlternateScreen).unwrap();
    disable_raw_mode().unwrap();
    mainkill.send();
    println!();
}

fn update(
    memory: &mut Memory,
    pc: &mut u16,
    display: &Arc<RwLock<[bool; 64 * 32]>>,
    sp: &mut u8,
    stack: &mut [u16; 16],
    registers: &mut Registers,
    shiftquirk: bool,
) {
    // NOTE: I think this should happen *before* an opcode update, as if the opcode sets the delay to
    // 8, we do not want to then decrement it immediately to 7, and instead wait until the next loop...
    // but have to check.
    let vdelay = registers.delay.load(Ordering::Acquire);
    if vdelay > 0 {
        registers.delay.store(vdelay - 1, Ordering::Release);
    }

    let vsound = registers.sound.load(Ordering::Acquire);
    if vsound > 0 {
        // TODO: Make actual sound
        registers.sound.store(vsound - 1, Ordering::Release);
    }

    let opcode = Cursor::new(&memory[*pc as usize..])
        .read_u16::<BigEndian>()
        .unwrap();
    info!("{:04x}: {:04x}", pc, opcode);
    *pc += 2;
    match opcode {
        // clear the screen
        0x00e0 => {
            let mut display = display.write().unwrap();
            for pixel in display.iter_mut() {
                *pixel = false;
            }
        }
        // return from subroutine
        0x00ee => {
            *sp -= 1;
            *pc = stack[*sp as usize];
        }
        // 0x1nnn - jump to address nnn
        0x1000..=0x1fff => {
            *pc = opcode & 0x0fff;
        }
        // 2nnn - call addr
        // call subroutine at nnn.
        // the interpreter increments the stack pointer, then puts the current pc on the top of the stack. the pc is then set to nnn.
        0x2000..=0x2fff => {
            stack[*sp as usize] = *pc;
            *sp += 1;
            *pc = opcode & 0x0fff;
        }
        // 3xkk - se vx, byte
        // skip next instruction if vx = kk.
        // the interpreter compares register vx to kk, and if they are equal, increments the program counter by 2.
        0x3000..=0x3fff => {
            let x = (opcode & 0x0f00) >> 8;
            let kk = (opcode & 0x00ff) as u8;
            if registers.v[x as usize] == kk {
                *pc += 2;
            }
        }
        //4xkk - sne vx, byte
        // skip next instruction if vx != kk.
        // the interpreter compares register vx to kk, and if they are not equal, increments the program counter by 2.
        0x4000..=0x4fff => {
            let x = (opcode & 0x0f00) >> 8;
            let kk = (opcode & 0x00ff) as u8;
            if registers.v[x as usize] != kk {
                *pc += 2;
            }
        }
        // 5xy0 - se vx, vy
        // skip next instruction if vx = vy.
        // the interpreter compares register vx to register vy, and if they are equal, increments the program counter by 2.
        0x5000..=0x5fff => {
            let x = (opcode & 0x0f00) >> 8;
            let y = (opcode & 0x00f0) >> 4;
            if x == y {
                *pc += 2;
            }
        }
        // set vx to nn
        0x6000..=0x6fff => {
            let x = (opcode & 0x0f00) >> 8;
            let nn = (opcode & 0x00ff) as u8;
            registers.v[x as usize] = nn;
        }
        0x7000..=0x7fff => {
            let x = (opcode & 0x0f00) >> 8;
            let kk = (opcode & 0x00ff) as u8;
            registers.v[x as usize] = registers.v[x as usize].wrapping_add(kk);
        }
        // 8xy0 - ld vx, vy
        // set vx = vy.
        // stores the value of register vy in register vx.
        0x8000..=0x8fff => {
            let x = (opcode & 0x0f00) >> 8;
            let y = (opcode & 0x00f0) >> 4;
            let op = opcode & 0x000f;
            match op {
                // ld vx, vy
                0x0 => {
                    registers.v[x as usize] = registers.v[y as usize];
                }
                // or vx, vy
                0x1 => {
                    registers.v[x as usize] |= registers.v[y as usize];
                }
                // and vx, vy
                0x2 => {
                    registers.v[x as usize] &= registers.v[y as usize];
                }
                // xor vx, vy
                0x3 => {
                    registers.v[x as usize] ^= registers.v[y as usize];
                }
                // add vx, vy
                0x4 => {
                    let (res, overflow) =
                        registers.v[x as usize].overflowing_add(registers.v[y as usize]);
                    registers.v[x as usize] = res;
                    registers.v[0xf] = overflow as u8;
                }
                // sub vx, vy
                0x5 => {
                    let (res, overflow) =
                        registers.v[x as usize].overflowing_sub(registers.v[y as usize]);
                    registers.v[x as usize] = res;
                    // not borrow
                    registers.v[0xf] = !overflow as u8;
                }
                // shr vx {, vy} ... todo will maybe have to revisit this
                0x6 => {
                    if !shiftquirk {
                        registers.v[x as usize] = registers.v[y as usize];
                    }
                    let flag = registers.v[x as usize] & 0x1;
                    registers.v[x as usize] >>= 1;
                    registers.v[0xf] = flag;
                }
                // subn vx, vy
                0x7 => {
                    let (res, overflow) =
                        registers.v[y as usize].overflowing_sub(registers.v[x as usize]);
                    registers.v[x as usize] = res;
                    registers.v[0xf] = !overflow as u8;
                }
                // shl vx {, vy}
                0xe => {
                    if !shiftquirk {
                        registers.v[x as usize] = registers.v[y as usize];
                    }
                    let flag = (registers.v[x as usize] & 0x80) >> 7;
                    registers.v[x as usize] <<= 1;
                    registers.v[0xf] = flag;
                }
                _ => panic!("unknown opcode instruction {:04x}", opcode),
            }
        }
        // 9xy0 - sne vx, vy
        // skip next instruction if vx != vy.
        // the values of vx and vy are compared, and if they are not equal, the program counter is increased by 2.
        0x9000..=0x9fff => {
            let x = (opcode & 0x0f00) >> 8;
            let y = (opcode & 0x00f0) >> 4;
            if registers.v[x as usize] != registers.v[y as usize] {
                *pc += 2;
            }
        }
        // set i to nnn
        0xa000..=0xafff => {
            registers.i = opcode & 0x0fff;
        }
        // dxyn - display n-byte sprite starting at memory location i at (vx, vy), set vf = collision.
        0xd000..=0xdfff => {
            let x = (opcode & 0x0f00) >> 8;
            let y = (opcode & 0x00f0) >> 4;
            let n = opcode & 0x000f;
            let vx = registers.v[x as usize] as usize;
            let vy = registers.v[y as usize] as usize;
            let mut collision = false;

            let mut display = display.write().unwrap();
            for byteidx in 0..n {
                let byte = memory[(registers.i + byteidx) as usize];
                for bitidx in 0..8 {
                    let bit = (byte >> (7 - bitidx)) & 1;
                    // wrap around the screen if needed
                    let idx = (vx + bitidx as usize) % 64 + ((vy + byteidx as usize) % 32) * 64;
                    if display[idx] && bit == 1 {
                        collision = true;
                    }
                    display[idx] ^= bit == 1;
                }
            }
            registers.v[0xf] = collision as u8;
        }
        0xf000..=0xffff => {
            let x = (opcode & 0x0f00) >> 8;
            let op = opcode & 0x00ff;
            match op {
                // fx07 - ld vx, dt
                // set vx = delay timer value.
                // the value of dt is placed into vx.
                0x07 => {
                    registers.v[x as usize] = registers.delay.load(Ordering::Acquire);
                }
                // fx0a - ld vx, k
                // wait for a key press, store the value of the key in vx.
                // all execution stops until a key is pressed, then the value of that key is stored in vx.
                0x0a => {
                    // todo: keypress
                }
                // fx15 - ld dt, vx
                // set delay timer = vx.
                // dt is set equal to the value of vx.
                0x15 => {
                    registers
                        .delay
                        .store(registers.v[x as usize], Ordering::Relaxed);
                }
                // fx18 - ld st, vx
                // set sound timer = vx.
                // st is set equal to the value of vx.
                0x18 => {
                    registers
                        .sound
                        .store(registers.v[x as usize], Ordering::Relaxed);
                }
                // fx1e - add i, vx
                // set i = i + vx.
                // the values of i and vx are added, and the results are stored in i.
                0x1e => {
                    registers.i += registers.v[x as usize] as u16;
                }
                // fx29 - ld f, vx
                // set i = location of sprite for digit vx.
                // the value of i is set to the location for the hexadecimal sprite corresponding to the value of vx.
                0x29 => {
                    // sprites are indexed from 0x0000 in memory
                    registers.i = registers.v[x as usize] as u16 * 5;
                }
                // fx33 - ld b, vx
                // store bcd representation of vx in memory locations i, i+1, and i+2.
                // the interpreter takes the decimal value of vx, and places the hundreds digit in memory at location in i,
                // the tens digit at location i+1, and the ones digit at location i+2.
                0x33 => {
                    let vx = registers.v[x as usize];
                    memory[registers.i as usize] = vx / 100;
                    memory[(registers.i + 1) as usize] = (vx / 10) % 10;
                    memory[(registers.i + 2) as usize] = vx % 10;
                }
                // fx55 - ld [i], vx
                // store registers v0 through vx in memory starting at location i.
                // the interpreter copies the values of registers v0 through vx into memory, starting at the address in i.
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
