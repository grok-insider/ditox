use rune::tok::{Family, TextStyle};
use rune::{Color, Icon, Shadow, oklch, oklcha};

pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
];
pub const FAMILIES: (&str, &str) = ("IBM Plex Sans", "IBM Plex Mono");

pub const RADIUS_THUMB: f32 = 3.0;
pub const RADIUS_CHECK: f32 = 4.0;
pub const RADIUS_TAG: f32 = 6.0;
pub const RADIUS_CONTROL: f32 = 8.0;
pub const RADIUS_MENU: f32 = 10.0;

/// One line of a clip in the list.
pub const ROW_LINE: f32 = 20.0;
pub const BODY: TextStyle = TextStyle::new(14.0, 400.0).lh(ROW_LINE);
pub const STRONG: TextStyle = TextStyle::new(14.0, 500.0).lh(20.0);
pub const CONTROL: TextStyle = TextStyle::new(13.0, 500.0).lh(18.0);
pub const META: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
pub const HEADING: TextStyle = TextStyle::new(15.0, 600.0).lh(22.0);
pub const LABEL: TextStyle = TextStyle::new(11.0, 400.0)
    .lh(16.0)
    .family(Family::Mono)
    .tracking(0.11)
    .features("'tnum' 1");
pub const CODE: TextStyle = TextStyle::new(13.0, 400.0).lh(20.0).family(Family::Mono);

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub page: Color,
    pub card: Color,
    pub well: Color,
    /// Every other row of the list.
    pub stripe: Color,
    pub hover: Color,
    pub border: Color,
    pub separator: Color,
    pub ink: Color,
    pub ink_muted: Color,
    pub brand_ink: Color,
    /// The fill of a checked box and of the button that saves.
    pub brand: Color,
    pub on_brand: Color,
    pub selected: Color,
    pub warning: Color,
    pub danger: Color,
}

/// `color-mix(in oklab, tone t, base)`, as the design tokens mix a tint.
fn tint(base: Color, tone: Color, t: f32) -> Color {
    let (base, tone) = (base.to_oklab(), tone.to_oklab());
    Color::from_oklab(std::array::from_fn(|channel| {
        base[channel] + (tone[channel] - base[channel]) * t
    }))
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        let (page, card, border) = (
            oklch(0.139, 0.006, 262.0),
            oklch(0.18, 0.016, 258.0),
            oklch(0.262, 0.021, 258.0),
        );
        let ink = oklch(0.934, 0.006, 129.0);
        Palette {
            dark,
            page,
            card,
            well: oklch(0.21, 0.019, 258.0),
            stripe: ink.alpha(0.022),
            hover: ink.alpha(0.05),
            border,
            separator: tint(page, border, 0.62),
            ink,
            ink_muted: oklch(0.69, 0.018, 255.0),
            brand_ink: oklch(0.813, 0.094, 258.0),
            brand: oklch(0.776, 0.098, 258.0),
            on_brand: oklch(0.177, 0.011, 261.0),
            selected: tint(page, oklch(0.776, 0.098, 258.0), 0.22),
            warning: oklch(0.84, 0.095, 82.0),
            danger: oklch(0.76, 0.115, 24.0),
        }
    } else {
        let (page, card, border) = (
            oklch(0.975, 0.004, 250.0),
            oklch(1.0, 0.0, 0.0),
            oklch(0.905, 0.011, 250.0),
        );
        let ink = oklch(0.177, 0.011, 261.0);
        Palette {
            dark,
            page,
            card,
            well: oklch(0.953, 0.009, 250.0),
            stripe: ink.alpha(0.022),
            hover: ink.alpha(0.05),
            border,
            separator: tint(page, border, 0.62),
            ink,
            ink_muted: oklch(0.47, 0.022, 258.0),
            brand_ink: oklch(0.481, 0.154, 260.0),
            brand: oklch(0.517, 0.163, 260.0),
            on_brand: oklch(1.0, 0.0, 0.0),
            selected: tint(page, oklch(0.517, 0.163, 260.0), 0.14),
            warning: oklch(0.5, 0.1, 70.0),
            danger: oklch(0.5, 0.15, 24.0),
        }
    }
}

impl Palette {
    /// The shadow under something that floats over the window: the menu.
    pub fn floating(&self) -> [Shadow; 2] {
        let (contact, drop) = if self.dark { (0.5, 0.45) } else { (0.08, 0.12) };
        [
            Shadow {
                x: 0.0,
                y: 1.0,
                blur: 2.0,
                spread: 0.0,
                color: oklcha(0.0, 0.0, 0.0, contact),
                inset: false,
            },
            Shadow {
                x: 0.0,
                y: 8.0,
                blur: 24.0,
                spread: 0.0,
                color: oklcha(0.0, 0.0, 0.0, drop),
                inset: false,
            },
        ]
    }
}

macro_rules! stroke_icon {
    ($name:ident, $id:literal, $body:literal) => {
        pub static $name: Icon = Icon {
            name: $id,
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        };
    };
}

stroke_icon!(
    ICON_SEARCH,
    "ditox-search",
    r#"<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>"#
);
stroke_icon!(
    ICON_CLOSE,
    "ditox-close",
    r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#
);
stroke_icon!(ICON_CHECK, "ditox-check", r#"<path d="M20 6 9 17l-5-5"/>"#);
stroke_icon!(
    ICON_MORE,
    "ditox-more",
    r#"<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>"#
);
