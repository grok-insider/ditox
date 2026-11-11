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

pub const RADIUS_KEY: f32 = 4.0;
pub const RADIUS_SWATCH: f32 = 4.0;
pub const RADIUS_TAG: f32 = 6.0;
pub const RADIUS_CONTROL: f32 = 8.0;
pub const RADIUS_PLATE: f32 = 12.0;

pub const BODY: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
pub const SEARCH: TextStyle = TextStyle::new(15.0, 400.0).lh(22.0);
pub const CONTROL: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
pub const META: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
pub const HEADING: TextStyle = TextStyle::new(16.0, 600.0).lh(24.0);
pub const LABEL: TextStyle = TextStyle::new(11.0, 400.0)
    .lh(16.0)
    .family(Family::Mono)
    .tracking(0.11)
    .features("'tnum' 1");
pub const KEY: TextStyle = TextStyle::new(11.0, 500.0).lh(16.0).family(Family::Mono);
pub const CODE: TextStyle = TextStyle::new(13.0, 400.0).lh(20.0).family(Family::Mono);

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub page: Color,
    pub card: Color,
    pub well: Color,
    pub hover: Color,
    pub border: Color,
    pub separator: Color,
    pub ink: Color,
    pub ink_muted: Color,
    pub brand_ink: Color,
    pub selected: Color,
    pub segment: Color,
    pub success: Color,
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
            hover: ink.alpha(0.045),
            border,
            separator: tint(page, border, 0.62),
            ink,
            ink_muted: oklch(0.69, 0.018, 255.0),
            brand_ink: oklch(0.813, 0.094, 258.0),
            selected: tint(page, oklch(0.776, 0.098, 258.0), 0.2),
            segment: oklch(0.285, 0.024, 258.0),
            success: oklch(0.8, 0.085, 168.0),
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
            hover: ink.alpha(0.045),
            border,
            separator: tint(page, border, 0.62),
            ink,
            ink_muted: oklch(0.47, 0.022, 258.0),
            brand_ink: oklch(0.481, 0.154, 260.0),
            selected: tint(page, oklch(0.517, 0.163, 260.0), 0.12),
            segment: card,
            success: oklch(0.48, 0.085, 168.0),
            warning: oklch(0.5, 0.1, 70.0),
            danger: oklch(0.5, 0.15, 24.0),
        }
    }
}

impl Palette {
    pub fn raised(&self) -> [Shadow; 2] {
        let (contact, drop) = if self.dark {
            (0.45, 0.35)
        } else {
            (0.06, 0.06)
        };
        [
            Shadow {
                x: 0.0,
                y: 1.0,
                blur: 1.0,
                spread: 0.0,
                color: oklcha(0.0, 0.0, 0.0, contact),
                inset: false,
            },
            Shadow {
                x: 0.0,
                y: 1.0,
                blur: 3.0,
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
    ICON_PIN,
    "ditox-pin",
    r#"<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#
);
stroke_icon!(
    ICON_IMAGE,
    "ditox-image",
    r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#
);
stroke_icon!(
    ICON_TEXT,
    "ditox-text",
    r#"<path d="M21 5H3"/><path d="M15 12H3"/><path d="M17 19H3"/>"#
);
stroke_icon!(
    ICON_LINK,
    "ditox-link",
    r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"#
);
stroke_icon!(
    ICON_OPEN,
    "ditox-open",
    r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#
);
stroke_icon!(
    ICON_CLOSE,
    "ditox-close",
    r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#
);
stroke_icon!(
    ICON_TRASH,
    "ditox-trash",
    r#"<path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/>"#
);
