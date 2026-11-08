//! Gizmos, which spin.
mod gizmo;

pub use gizmo::Gizmo;

/// A new gizmo's turns after spinning it twice.
pub fn spin_twice() -> u32 {
    let mut gizmo = Gizmo::new();
    gizmo.spin();
    gizmo.spin();
    gizmo.turns()
}
