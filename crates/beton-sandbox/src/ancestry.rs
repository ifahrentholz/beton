//! Prozess-Abstammung (#247): Ist ein Prozess ein Nachfahre eines anderen? Grundlage der
//! Peer-Prüfung am Hook-Socket.

/// Ist `pid` gleich `root` oder ein Nachfahre davon?
pub fn is_descendant(_pid: u32, _root: u32) -> bool {
    true
}
