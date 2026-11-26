//! Platform-independent policy for choosing a desktop blur effect.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Mode { Disabled, Classic, Acrylic }
/// Respect accessibility/transparency settings; older Windows keeps classic blur.
pub fn choose(enabled:bool,transparency:bool,acrylic:bool,build:u32)->Mode {
    if !enabled || !transparency {Mode::Disabled}
    else if acrylic && build>=17134 {Mode::Acrylic} else {Mode::Classic}
}
/// Keep the saved value intact, but render opaquely when Windows disables transparency.
pub fn effective_opacity(requested:u8,transparency:bool)->u8 {
    if transparency {requested.clamp(10,100)} else {100}
}
#[cfg(test)] mod tests {
    use super::*;
    #[test]fn opaque_accessibility_fallback_does_not_require_changing_the_profile(){
        assert_eq!(effective_opacity(35,true),35);assert_eq!(effective_opacity(35,false),100);
        assert_eq!(effective_opacity(0,true),10);assert_eq!(effective_opacity(255,true),100);
    }
    #[test]fn follows_windows_accessibility_and_falls_back_on_old_versions(){
        assert_eq!(choose(true,false,true,22631),Mode::Disabled);
        assert_eq!(choose(false,true,true,22631),Mode::Disabled);
        assert_eq!(choose(true,true,true,15063),Mode::Classic);
        assert_eq!(choose(true,true,false,22631),Mode::Classic);
        assert_eq!(choose(true,true,true,22631),Mode::Acrylic);
    }
}
