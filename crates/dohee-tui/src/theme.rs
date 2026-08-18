use ratatui::style::{Color, Style, Modifier};

pub struct Theme {
    pub c_cyan: Color,
    pub c_green: Color,
    pub c_yellow: Color,
    pub c_blue: Color,
    pub c_magenta: Color,
    pub c_subtext: Color,
    pub c_header_bg: Color,
    pub c_composer_bg: Color,
    pub c_footer_bg: Color,
    
    pub style_user: Style,
    pub style_assistant: Style,
    pub style_system: Style,
    pub style_tool: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let c_cyan = Color::Rgb(137, 220, 235);
        let c_green = Color::Rgb(166, 227, 161);
        let c_yellow = Color::Rgb(249, 226, 175);
        let c_blue = Color::Rgb(137, 180, 250);
        let c_magenta = Color::Rgb(203, 166, 247);
        let c_subtext = Color::Rgb(108, 112, 134);
        let c_header_bg = Color::Rgb(24, 24, 37);
        let c_composer_bg = Color::Rgb(30, 30, 46);
        let c_footer_bg = Color::Rgb(17, 17, 27);

        Self {
            c_cyan,
            c_green,
            c_yellow,
            c_blue,
            c_magenta,
            c_subtext,
            c_header_bg,
            c_composer_bg,
            c_footer_bg,
            
            style_user: Style::default().fg(c_cyan).add_modifier(Modifier::BOLD),
            style_assistant: Style::default().fg(c_green),
            style_system: Style::default().fg(c_subtext),
            style_tool: Style::default().fg(c_yellow),
        }
    }
}
