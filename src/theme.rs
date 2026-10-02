use ratatui::style::Color;

/// A full color palette. Field names follow Catppuccin's naming so every theme
/// maps onto the same semantic slots.
#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub crust: Color,
    pub mantle: Color,
    pub base: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub surface2: Color,
    pub overlay0: Color,
    pub overlay1: Color,
    pub subtext: Color,
    pub text: Color,
    pub lavender: Color,
    pub blue: Color,
    pub sapphire: Color,
    pub sky: Color,
    pub teal: Color,
    pub green: Color,
    pub yellow: Color,
    pub peach: Color,
    pub maroon: Color,
    pub red: Color,
    pub mauve: Color,
    pub pink: Color,
}

const fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const THEMES: &[Theme] = &[
    Theme {
        name: "Catppuccin Mocha",
        crust: hex(0x11111b),
        mantle: hex(0x181825),
        base: hex(0x1e1e2e),
        surface0: hex(0x313244),
        surface1: hex(0x45475a),
        surface2: hex(0x585b70),
        overlay0: hex(0x6c7086),
        overlay1: hex(0x7f849c),
        subtext: hex(0xa6adc8),
        text: hex(0xcdd6f4),
        lavender: hex(0xb4befe),
        blue: hex(0x89b4fa),
        sapphire: hex(0x74c7ec),
        sky: hex(0x89dceb),
        teal: hex(0x94e2d5),
        green: hex(0xa6e3a1),
        yellow: hex(0xf9e2af),
        peach: hex(0xfab387),
        maroon: hex(0xeba0ac),
        red: hex(0xf38ba8),
        mauve: hex(0xcba6f7),
        pink: hex(0xf5c2e7),
    },
    Theme {
        name: "Tokyo Night",
        crust: hex(0x16161e),
        mantle: hex(0x1a1b26),
        base: hex(0x1a1b26),
        surface0: hex(0x24283b),
        surface1: hex(0x2f3549),
        surface2: hex(0x414868),
        overlay0: hex(0x565f89),
        overlay1: hex(0x737aa2),
        subtext: hex(0xa9b1d6),
        text: hex(0xc0caf5),
        lavender: hex(0xb4f9f8),
        blue: hex(0x7aa2f7),
        sapphire: hex(0x2ac3de),
        sky: hex(0x7dcfff),
        teal: hex(0x73daca),
        green: hex(0x9ece6a),
        yellow: hex(0xe0af68),
        peach: hex(0xff9e64),
        maroon: hex(0xdb4b4b),
        red: hex(0xf7768e),
        mauve: hex(0xbb9af7),
        pink: hex(0xff007c),
    },
    Theme {
        name: "Nord",
        crust: hex(0x242933),
        mantle: hex(0x2b303b),
        base: hex(0x2e3440),
        surface0: hex(0x3b4252),
        surface1: hex(0x434c5e),
        surface2: hex(0x4c566a),
        overlay0: hex(0x616e88),
        overlay1: hex(0x7b88a1),
        subtext: hex(0xd8dee9),
        text: hex(0xeceff4),
        lavender: hex(0xb48ead),
        blue: hex(0x81a1c1),
        sapphire: hex(0x5e81ac),
        sky: hex(0x88c0d0),
        teal: hex(0x8fbcbb),
        green: hex(0xa3be8c),
        yellow: hex(0xebcb8b),
        peach: hex(0xd08770),
        maroon: hex(0xbf616a),
        red: hex(0xbf616a),
        mauve: hex(0xb48ead),
        pink: hex(0xd8a0c8),
    },
    Theme {
        name: "Dracula",
        crust: hex(0x191a21),
        mantle: hex(0x21222c),
        base: hex(0x282a36),
        surface0: hex(0x343746),
        surface1: hex(0x44475a),
        surface2: hex(0x565970),
        overlay0: hex(0x6272a4),
        overlay1: hex(0x7c86b4),
        subtext: hex(0xbfbfbf),
        text: hex(0xf8f8f2),
        lavender: hex(0xd6acff),
        blue: hex(0x8be9fd),
        sapphire: hex(0x62d6e8),
        sky: hex(0xa4ffff),
        teal: hex(0x69ff94),
        green: hex(0x50fa7b),
        yellow: hex(0xf1fa8c),
        peach: hex(0xffb86c),
        maroon: hex(0xff6e6e),
        red: hex(0xff5555),
        mauve: hex(0xbd93f9),
        pink: hex(0xff79c6),
    },
];

pub fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}

pub fn mix(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let l = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    Color::Rgb(l(ar, br), l(ag, bg), l(ab, bb))
}

/// Samples a multi-stop gradient at `t` in 0..=1.
pub fn gradient(stops: &[Color], t: f64) -> Color {
    match stops.len() {
        0 => Color::Reset,
        1 => stops[0],
        n => {
            let t = t.clamp(0.0, 1.0) * (n - 1) as f64;
            let i = (t.floor() as usize).min(n - 2);
            mix(stops[i], stops[i + 1], t - i as f64)
        }
    }
}

impl Theme {
    pub fn cpu_gradient(&self) -> [Color; 4] {
        [self.green, self.yellow, self.peach, self.red]
    }
    pub fn mem_gradient(&self) -> [Color; 3] {
        [self.lavender, self.mauve, self.pink]
    }
    pub fn net_down_gradient(&self) -> [Color; 3] {
        [self.sapphire, self.sky, self.teal]
    }
    pub fn net_up_gradient(&self) -> [Color; 3] {
        [self.mauve, self.pink, self.maroon]
    }
    pub fn disk_read_gradient(&self) -> [Color; 3] {
        [self.teal, self.green, self.yellow]
    }
    pub fn disk_write_gradient(&self) -> [Color; 3] {
        [self.peach, self.maroon, self.red]
    }
    pub fn heat_gradient(&self) -> [Color; 5] {
        [self.sapphire, self.green, self.yellow, self.peach, self.red]
    }

    /// Color for a percentage on the green→red load scale.
    pub fn load_color(&self, pct: f64) -> Color {
        gradient(&self.cpu_gradient(), pct / 100.0)
    }

    /// Color for a temperature in °C.
    pub fn temp_color(&self, c: f64) -> Color {
        gradient(&self.heat_gradient(), (c - 25.0) / 75.0)
    }
}
