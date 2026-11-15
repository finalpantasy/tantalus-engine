//! World coordinates, block and cell addressing.
//!
//! Unity units: a block is 64 units square, a cell 32 (`WMConstants`). World x grows
//! east and world z grows toward the south, so block row and cell row are read from
//! `-z`. Fixed point is 1/256 unit (`WMConstants.Denominator`).

use super::vec3::Vec3;

/// `WMConstants.WorldBlockFigureX` (`WM/WMConstants.cs:11`).
pub const BLOCK_COLUMNS: usize = 24;
/// `WMConstants.WorldBlockFigureZ` (`WM/WMConstants.cs:13`).
pub const BLOCK_ROWS: usize = 20;
/// `WMConstants.WorldCellFigureX` (`WM/WMConstants.cs:15`).
pub const CELL_COLUMNS: i32 = 48;
/// `WMConstants.WorldCellFigureZ` (`WM/WMConstants.cs:17`).
pub const CELL_ROWS: i32 = 40;
/// `WMConstants.BlockDistance` (`WM/WMConstants.cs:21`).
pub const BLOCK_DISTANCE: f32 = 64.0;
/// `WMConstants.CellDistance` (`WM/WMConstants.cs:25`).
pub const CELL_DISTANCE: f32 = 32.0;
/// `WMConstants.OriginalBlockDistance` (`WM/WMConstants.cs:19`).
pub const ORIGINAL_BLOCK_DISTANCE: i32 = 16384;

/// `ff9.S` (`ff9/ff9.cs:2403`): fixed point to Unity units.
pub fn fixed_to_unity(fixed_point: i32) -> f32 {
    fixed_point as f32 * 0.003_906_25
}

/// `ff9.UnityUnit` (`ff9/ff9.cs:2405`): Unity units to fixed point (truncating).
pub fn unity_to_fixed(value: f32) -> i32 {
    (value * 256.0) as i32
}

/// `ff9.PsxRot` (`ff9/ff9.cs:2412`): 4096 steps per turn to degrees.
pub fn psx_rot(angle: i32) -> f32 {
    angle as f32 * 0.087_890_625
}

/// `ff9.m_GetIDEvent` (`ff9/ff9.cs:2332`): bits 14 and 15 of a map id.
pub fn id_event(id: i32) -> i32 {
    (id & 0xC000) >> 14
}

/// `ff9.m_GetIDArea` (`ff9/ff9.cs:2337`): bits 8 to 13 of a map id.
pub fn id_area(id: i32) -> i32 {
    (id & 0x3F00) >> 8
}

/// `ff9.m_GetIDTopograph` (`ff9/ff9.cs:2342`): bits 2 to 7 of a map id.
pub fn id_topograph(id: i32) -> i32 {
    (id & 0xFC) >> 2
}

/// Block grid indices `(x, z)` of a world position, as `WMWorld.GetAbsoluteBlock`
/// reads them (`WM/WMWorld/WMWorld.cs:299`): `(int)(x / 64)` and
/// `(int)(|z| / 64)`. The source returns null for `x >= 24` or `z >= 20`; a
/// negative `x` index would throw there, so it is `None` here.
pub fn block_index(x: f32, z: f32) -> Option<(usize, usize)> {
    let bx = (x / BLOCK_DISTANCE) as i32;
    let bz = (z.abs() / BLOCK_DISTANCE) as i32;
    if bx < 0 || bx >= BLOCK_COLUMNS as i32 || bz >= BLOCK_ROWS as i32 {
        return None;
    }
    Some((bx as usize, bz as usize))
}

/// Cell column and row of a world position (`ff9.w_worldPos2Cell`,
/// `ff9/ff9.cs:8941`): `(int)(x / 32)` and `(int)(z / -32)`.
pub fn cell_xz(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL_DISTANCE) as i32, (z / -CELL_DISTANCE) as i32)
}

/// Linear cell number `row * 48 + column` (`ff9.w_worldPos2Cell`).
pub fn cell_index(x: f32, z: f32) -> i32 {
    let (cx, cz) = cell_xz(x, z);
    cz * CELL_COLUMNS + cx
}

/// Block-to-block mapping that `WMWorld` keeps while it wraps the world
/// (`WMBlock.InitialX/Y`, `CurrentX/Y`). The identity layout is the unshifted
/// world; `Wrap` and `ShiftBlocks` (`WMWorld.cs:1173`, `:1389`) are not ported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLayout {
    initial: Vec<(u8, u8)>,
    current: Vec<(u8, u8)>,
}

impl BlockLayout {
    pub fn identity() -> Self {
        let table: Vec<(u8, u8)> = (0..BLOCK_COLUMNS * BLOCK_ROWS)
            .map(|i| ((i / BLOCK_ROWS) as u8, (i % BLOCK_ROWS) as u8))
            .collect();
        Self {
            initial: table.clone(),
            current: table,
        }
    }

    fn slot(x: usize, z: usize) -> usize {
        x * BLOCK_ROWS + z
    }

    /// Sets `Blocks[x, z]`'s `InitialX/Y` and `CurrentX/Y`.
    pub fn set(&mut self, x: usize, z: usize, initial: (u8, u8), current: (u8, u8)) {
        self.initial[Self::slot(x, z)] = initial;
        self.current[Self::slot(x, z)] = current;
    }

    /// `WMWorld.GetAbsoluteBlock` (`WMWorld.cs:299`): the block whose grid position
    /// holds `position`, resolved through `Blocks[CurrentX, CurrentY]`.
    pub fn absolute_block(&self, position: Vec3) -> Option<(usize, usize)> {
        let (x, z) = block_index(position.x, position.z)?;
        let (cx, cy) = self.current[Self::slot(x, z)];
        Some((usize::from(cx), usize::from(cy)))
    }

    /// `WMWorld.GetAbsolutePositionOf(Vector3)` (`WMWorld.cs:253`).
    pub fn absolute_position_of(&self, position: Vec3) -> Option<Vec3> {
        let local_x = position.x % BLOCK_DISTANCE;
        let local_z = position.z % BLOCK_DISTANCE;
        let (x, z) = block_index(position.x, position.z)?;
        let (ix, iy) = self.initial[Self::slot(x, z)];
        Some(Vec3::new(
            f32::from(ix) * 64.0 + local_x,
            position.y,
            -f32::from(iy) * 64.0 + local_z,
        ))
    }

    /// `WMWorld.GetAbsolutePositionOf_FixedPoint` (`WMWorld.cs:266`) on x and z.
    pub fn absolute_position_fixed(&self, x: i32, z: i32) -> Option<(i32, i32)> {
        let local_x = x % ORIGINAL_BLOCK_DISTANCE;
        let local_z = z % ORIGINAL_BLOCK_DISTANCE;
        let bx = x / ORIGINAL_BLOCK_DISTANCE;
        let bz = z.abs() / ORIGINAL_BLOCK_DISTANCE;
        if bx < 0 || bx >= BLOCK_COLUMNS as i32 || bz < 0 || bz >= BLOCK_ROWS as i32 {
            return None;
        }
        let (ix, iy) = self.initial[Self::slot(bx as usize, bz as usize)];
        Some((
            i32::from(ix) * ORIGINAL_BLOCK_DISTANCE + local_x,
            -i32::from(iy) * ORIGINAL_BLOCK_DISTANCE + local_z,
        ))
    }
}

/// World translation of block `(column, row)` (`WMWorld.OnInitialize`,
/// `WMWorld.cs:455`): `x = column * 64`, `z = -row * 64`.
pub fn block_origin(column: usize, row: usize) -> Vec3 {
    Vec3::new(
        (column as i32 * ORIGINAL_BLOCK_DISTANCE) as f32 * 0.003_906_25,
        0.0,
        (row as i32 * -ORIGINAL_BLOCK_DISTANCE) as f32 * 0.003_906_25,
    )
}
