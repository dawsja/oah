use opentui::*;
use std::time::Duration;

fn main() {
    let mut t = Terminal::new(TerminalOptions::default()).expect("terminal");
    let mut last = String::from("press keys, q quits");
    loop {
        {
            let w = t.width() as u32;
            let h = t.height() as u32;
            let mut f = t.frame();
            f.clear(Rgba::hex(0x101014));
            f.boxed(0, 0, w, h, BorderStyle::Rounded, Rgba::hex(0x5a5a6a), None, Some((" opentui-rs ", Align::Left, Rgba::hex(0xc0a0ff))), None);
            f.text(2, 2, "Hello from Rust → OpenTUI ✓ 世界", Rgba::hex(0xffffff), None, attr::BOLD, 0);
            f.text(2, 4, &last, Rgba::hex(0x9adf9a), None, 0, w - 4);
        }
        t.present().unwrap();
        for ev in t.poll(Duration::from_millis(100)) {
            if let Event::Key(k) = &ev { if k.text == "q" { return; } }
            last = format!("{:?}", ev);
        }
    }
}
