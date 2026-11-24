//! Tela de configurações (provisória).

use objc2::MainThreadMarker;
use objc2::runtime::AnyObject;

pub fn open(_mtm: MainThreadMarker) {}
pub fn frame() {}
pub fn refresh() {}
pub fn control_changed(_sender: Option<&AnyObject>) {}
pub fn button(_sender: Option<&AnyObject>) {}
