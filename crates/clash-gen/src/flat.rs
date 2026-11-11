//! The `flat` test generator: ground and nothing else (02 §9.3).

use std::sync::Arc;

use clash_world::{
    BlockState, ColumnBuilder, FloorState, GenCtx, GenError, GeneratorRequest, RealizedTract,
    RealmInfo, RegionPos, RegionRecord, RegionSurvey, RegionSurveyResult, RegionSurveyor, TractPos,
    TractRealizer, TractRecord, TractRecords3x3, WorldGenerator,
};

/// The name realm definitions give this generator by.
pub const FLAT_GENERATOR: &str = "flat";

/// The parameter that gives the storey of the surface. Default 0.
const SURFACE_Z: &str = "surface_z";
/// The parameter that names the block the ground is made of. Default:
/// what the realm's `fill` says lies below.
const GROUND: &str = "ground";
/// The parameter that names the floor laid over the surface. Default:
/// none.
const FLOOR: &str = "floor";

/// Flat ground: one block state in every storey below the surface,
/// nothing from there up, and optionally a floor laid over the surface.
///
/// | Parameter | Meaning | Default |
/// |-----------|---------|---------|
/// | `surface_z` | The lowest storey that is not ground | 0 |
/// | `ground` | The block state the ground is made of | The realm's `fill.below` |
/// | `floor` | The floor state laid over the surface | None |
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlatGenerator {
    /// The lowest storey that is not ground.
    pub surface_z: i32,
    /// What the ground is made of.
    pub ground: BlockState,
    /// The floor laid over the surface, or none.
    pub floor: FloorState,
}

impl FlatGenerator {
    /// Makes the generator a realm definition asks for.
    pub fn factory(request: &GeneratorRequest<'_>) -> Result<Arc<dyn WorldGenerator>, GenError> {
        request.only(&[SURFACE_Z, GROUND, FLOOR])?;
        let topology = request.kind.topology;
        let surface_z = request.int(
            SURFACE_Z,
            0,
            i64::from(topology.z_min)..=i64::from(topology.z_max),
        )?;
        let has = |param: &str| request.kind.generator_params.contains_key(param);
        let ground = if has(GROUND) {
            request.block(GROUND)?
        } else {
            request.kind.below_fill
        };
        let floor = if has(FLOOR) {
            request.floor(FLOOR)?
        } else {
            FloorState::NONE
        };
        Ok(Arc::new(FlatGenerator {
            // Within the realm's storeys, which are `i32`s.
            surface_z: i32::try_from(surface_z).unwrap_or(0),
            ground,
            floor,
        }))
    }
}

impl RegionSurveyor for FlatGenerator {
    fn survey(&self, _: &GenCtx<'_>, _: &RealmInfo, _: RegionPos) -> RegionSurveyResult {
        RegionSurveyResult::new(RegionRecord::new(RegionSurvey::default()))
    }
}

impl TractRealizer for FlatGenerator {
    fn realize(
        &self,
        cx: &GenCtx<'_>,
        realm: &RealmInfo,
        _: TractPos,
        _: &TractRecord,
        _: &TractRecords3x3,
    ) -> RealizedTract {
        let mut column =
            ColumnBuilder::new(cx.defs(), self.ground, realm.above_fill, self.surface_z);
        if !self.floor.is_none() {
            column.fill_floor(self.surface_z, self.floor);
        }
        RealizedTract::new(column.build())
    }
}
