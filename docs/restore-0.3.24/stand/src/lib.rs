// The module under test is used by the app through callers this bench does not have,
// so anything only reachable from there reads as dead code here.
#![allow(dead_code)]

pub mod request;
pub mod aura_core { pub mod i18n { pub fn text(s:&str)->&str{s} } }
pub mod aura_windows {
    pub mod desktop {
        #[derive(Debug, Clone)]
        pub enum MenuRow<'a> {
            Command { label: std::borrow::Cow<'a, str>, identity: std::borrow::Cow<'a, str>, checked: bool, enabled: bool },
            Submenu { label: std::borrow::Cow<'a, str>, items: Vec<MenuRow<'a>> },
            Separator,
        }
    }
    pub mod shell {
        pub mod file_transfer {
            pub fn paths_from_pidls(p:&[Vec<u8>])->Result<Vec<String>,()>{
                if p.iter().any(|b| b.is_empty()) { return Err(()) }
                Ok(p.iter().enumerate().map(|(i,_)|format!("C:/item{i}")).collect())
            }
        }
        pub mod menu {
            #[derive(Default, Debug, Clone)]
            pub struct ContextMenuOptions { pub modern: bool, pub drawn: bool, pub hidden_verbs: Vec<String>, pub shown_verbs: Vec<String> }
            pub fn common_command_visible(o:&ContextMenuOptions,verb:&str,label:&str)->bool{
                let _=label;
                !o.hidden_verbs.iter().any(|h|h==verb)
            }
            #[derive(Debug, PartialEq, Eq, Clone)]
            pub enum MenuOutcome { Dismissed, ShellInvoked, Application(u32), CopyPath, Compress, Extract }
        }
    }
}

pub fn archive_among(paths:&[String])->Option<&String>{
    paths.iter().find(|p|p.ends_with(".zip"))
}

pub mod common;
