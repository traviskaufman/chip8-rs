# Rust CHIP-8 Emulator

[CHIP-8](https://en.wikipedia.org/wiki/CHIP-8) emulator in rust, which renders graphics to the terminal.

<img width="1440" alt="image" src="https://github.com/traviskaufman/chip8-rs/assets/1185269/8ece16d2-5d4f-4d26-9b6e-526a9825e774">

This was a cute little side project I did on a day off. I've always been interested in emu dev and I thought it would be cool to make one with 8-bit graphics that goes directly to your terminal. It's pretty primitive but it can render the test ROM in `roms/`. It has pretty much no extrinsic value and there are way better CHIP-8 emus out there, but it was the most fun I've had coding in a very long time.

Huge shout-out to:

- Thomas P. Green for his [CHIP-8 Reference](http://devernay.free.fr/hacks/chip8/C8TECH10.HTM#5.0)
- @Timendus for his [CHIP-8 Test Suite](https://github.com/Timendus/chip8-test-suite?tab=readme-ov-file)
- GitHub Copilot for writing most of the opcode parsing logic :smiley:
