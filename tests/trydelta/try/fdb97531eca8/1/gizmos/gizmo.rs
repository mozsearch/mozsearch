/// A gizmo, which counts its turns.
pub struct Gizmo {
    turns: u32,
}

impl Gizmo {
    pub fn new() -> Gizmo {
        Gizmo { turns: 0 }
    }

    /// Twirl the gizmo once.
    pub fn twirl(&mut self) {
        self.turns += 1;
    }

    pub fn turns(&self) -> u32 {
        self.turns
    }
}
