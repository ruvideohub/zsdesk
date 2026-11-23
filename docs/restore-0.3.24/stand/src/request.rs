use crate::aura_windows::shell::menu::ContextMenuOptions;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode { More, Background, New, Open, Properties }

#[derive(Debug)]
pub struct Request {
    pub extended: bool,
    pub mode: Mode,
    pub file_actions: bool,
    pub pidls: Vec<Vec<u8>>,
    pub folder: Option<String>,
    pub options: ContextMenuOptions,
}
impl Default for Request {
    fn default()->Self{ Self{ extended:false, mode:Mode::More, file_actions:true, pidls:Vec::new(), folder:None, options:ContextMenuOptions::default() } }
}
