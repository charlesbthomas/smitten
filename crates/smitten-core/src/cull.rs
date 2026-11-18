/// Which way round a triangle's corners go on screen, the equivalent of
/// LÖVE's `VertexWinding`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VertexWinding {
    /// Clockwise as seen on screen, with y pointing down.
    Clockwise,
    /// Counter-clockwise as seen on screen, with y pointing down. LÖVE's
    /// default front face.
    #[default]
    CounterClockwise,
}

impl VertexWinding {
    /// The other way round.
    pub fn reversed(self) -> VertexWinding {
        match self {
            VertexWinding::Clockwise => VertexWinding::CounterClockwise,
            VertexWinding::CounterClockwise => VertexWinding::Clockwise,
        }
    }
}

/// Which triangles of a mesh are left out, the equivalent of LÖVE's
/// `CullMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CullMode {
    /// Every triangle is drawn. The default.
    #[default]
    None,
    /// Triangles facing away, whose corners go the other way from the
    /// front face, are left out.
    Back,
    /// Triangles facing the viewer are left out.
    Front,
}

impl CullMode {
    /// The on-screen winding of the triangles to leave out, given which
    /// winding faces the viewer.
    pub fn culled(self, front_face: VertexWinding) -> Option<VertexWinding> {
        match self {
            CullMode::None => None,
            CullMode::Back => Some(front_face.reversed()),
            CullMode::Front => Some(front_face),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_faces_are_the_other_winding_from_the_front() {
        let front = VertexWinding::CounterClockwise;
        assert_eq!(CullMode::Back.culled(front), Some(VertexWinding::Clockwise));
        assert_eq!(CullMode::Front.culled(front), Some(front));
        assert_eq!(CullMode::None.culled(front), None);
    }
}
