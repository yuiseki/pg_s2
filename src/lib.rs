use pgrx::callconv::{ArgAbi, BoxRet};
use pgrx::datum::Datum;
use pgrx::guc::{GucContext, GucFlags, GucRegistry, GucSetting};
use pgrx::iter::SetOfIterator;
use pgrx::pg_sys::Point;
use pgrx::pg_sys::Oid;
use pgrx::pgrx_sql_entity_graph::metadata::{
    ArgumentError, Returns, ReturnsError, SqlMapping, SqlTranslatable,
};
use pgrx::prelude::*;
use pgrx::{rust_regtypein, StringInfo};
use s2::cap::Cap;
use s2::cell::Cell;
use s2::cellid::{CellID, NUM_FACES, POS_BITS};
use s2::latlng::LatLng;
use s2::point::Point as S2Point;
use s2::region::RegionCoverer;
use s2::rect::Rect;
use s2::s1::{Angle, Rad};
use std::ffi::CStr;

::pgrx::pg_module_magic!(name, version);

const S2CELLID_ORDER_MASK: u64 = 0x8000_0000_0000_0000;
const S2CELLID_LSB_MASK: u64 = 0x1555_5555_5555_5555;
const DEFAULT_MAX_CELLS: i32 = 8;
const EARTH_RADIUS_M_DEFAULT: f64 = 6_371_008.8;
static DEFAULT_LEVEL: GucSetting<i32> = GucSetting::<i32>::new(14);
static EARTH_RADIUS_M: GucSetting<f64> = GucSetting::<f64>::new(EARTH_RADIUS_M_DEFAULT);
static DEFAULT_LEVEL_NAME: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"pg_s2.default_level\0") };
static DEFAULT_LEVEL_SHORT: &CStr = unsafe {
    CStr::from_bytes_with_nul_unchecked(b"Default S2 level for s2_lat_lng_to_cell(point).\0")
};
static DEFAULT_LEVEL_DESC: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"Used when level is not explicitly provided.\0") };
static EARTH_RADIUS_M_NAME: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"pg_s2.earth_radius_m\0") };
static EARTH_RADIUS_M_SHORT: &CStr = unsafe {
    CStr::from_bytes_with_nul_unchecked(b"Earth radius in meters for distance and cap conversions.\0")
};
static EARTH_RADIUS_M_DESC: &CStr = unsafe {
    CStr::from_bytes_with_nul_unchecked(
        b"Used to convert between meters and radians in s2_great_circle_distance and s2_cover_cap.\0",
    )
};
static DEFAULT_COVER_LEVEL: GucSetting<i32> = GucSetting::<i32>::new(12);
static DEFAULT_COVER_LEVEL_NAME: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"pg_s2.default_cover_level\0") };
static DEFAULT_COVER_LEVEL_SHORT: &CStr = unsafe {
    CStr::from_bytes_with_nul_unchecked(b"Default S2 level for s2_cover_rect(point).\0")
};
static DEFAULT_COVER_LEVEL_DESC: &CStr = unsafe {
    CStr::from_bytes_with_nul_unchecked(b"Used when cover level is not explicitly provided.\0")
};

#[pg_guard]
pub extern "C-unwind" fn _PG_init() {
    GucRegistry::define_int_guc(
        DEFAULT_LEVEL_NAME,
        DEFAULT_LEVEL_SHORT,
        DEFAULT_LEVEL_DESC,
        &DEFAULT_LEVEL,
        0,
        30,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_float_guc(
        EARTH_RADIUS_M_NAME,
        EARTH_RADIUS_M_SHORT,
        EARTH_RADIUS_M_DESC,
        &EARTH_RADIUS_M,
        0.0,
        1.0e9,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        DEFAULT_COVER_LEVEL_NAME,
        DEFAULT_COVER_LEVEL_SHORT,
        DEFAULT_COVER_LEVEL_DESC,
        &DEFAULT_COVER_LEVEL,
        0,
        30,
        GucContext::Userset,
        GucFlags::default(),
    );
}

#[inline]
fn u64_to_i64_norm(cellid: u64) -> i64 {
    (cellid ^ S2CELLID_ORDER_MASK) as i64
}

#[inline]
fn i64_norm_to_u64(norm: i64) -> u64 {
    (norm as u64) ^ S2CELLID_ORDER_MASK
}

#[inline]
fn s2_cellid_is_valid_raw(raw: u64) -> bool {
    let face = (raw >> POS_BITS) as u8;
    if face >= NUM_FACES {
        return false;
    }
    let lsb = raw & raw.wrapping_neg();
    (lsb & S2CELLID_LSB_MASK) != 0
}

#[repr(transparent)]
#[derive(
    Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash, PostgresEq, PostgresOrd, PostgresHash,
)]
pub struct S2CellId {
    value: i64,
}

impl S2CellId {
    #[inline]
    fn from_u64(cellid: u64) -> Self {
        Self {
            value: u64_to_i64_norm(cellid),
        }
    }

    #[inline]
    fn to_u64(self) -> u64 {
        i64_norm_to_u64(self.value)
    }
}

impl std::fmt::Display for S2CellId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let token = CellID(self.to_u64()).to_token();
        write!(f, "{token}")
    }
}

unsafe impl SqlTranslatable for S2CellId {
    fn argument_sql() -> Result<SqlMapping, ArgumentError> {
        Ok(SqlMapping::As("s2cellid".into()))
    }

    fn return_sql() -> Result<Returns, ReturnsError> {
        Ok(Returns::One(SqlMapping::As("s2cellid".into())))
    }
}

impl FromDatum for S2CellId {
    unsafe fn from_polymorphic_datum(datum: pg_sys::Datum, is_null: bool, _: Oid) -> Option<Self>
    where
        Self: Sized,
    {
        if is_null {
            None
        } else {
            Some(S2CellId {
                value: datum.value() as _,
            })
        }
    }
}

impl IntoDatum for S2CellId {
    fn into_datum(self) -> Option<pg_sys::Datum> {
        Some(pg_sys::Datum::from(self.value))
    }

    fn type_oid() -> Oid {
        rust_regtypein::<Self>()
    }
}

unsafe impl<'fcx> ArgAbi<'fcx> for S2CellId
where
    Self: 'fcx,
{
    unsafe fn unbox_arg_unchecked(arg: ::pgrx::callconv::Arg<'_, 'fcx>) -> Self {
        arg.unbox_arg_using_from_datum().unwrap()
    }
}

unsafe impl BoxRet for S2CellId {
    unsafe fn box_into<'fcx>(self, fcinfo: &mut pgrx::callconv::FcInfo<'fcx>) -> Datum<'fcx> {
        fcinfo.return_raw_datum(pg_sys::Datum::from(self.value))
    }
}

#[pg_extern(immutable, parallel_safe, requires = ["shell_type"])]
fn s2cellid_in(input: &CStr) -> S2CellId {
    let token = input
        .to_str()
        .unwrap_or_else(|_| error!("invalid s2cellid token"));
    let cellid = CellID::from_token(token);
    if !s2_cellid_is_valid_raw(cellid.0) {
        error!("invalid s2cellid token");
    }
    S2CellId::from_u64(cellid.0)
}

#[pg_extern(immutable, parallel_safe, requires = ["shell_type"])]
fn s2cellid_out(value: S2CellId) -> &'static CStr {
    let mut s = StringInfo::new();
    s.push_str(&value.to_string());
    unsafe { s.leak_cstr() }
}

extension_sql!(
    r#"
CREATE TYPE s2cellid;
"#,
    name = "shell_type",
    bootstrap
);

extension_sql!(
    r#"
CREATE TYPE s2cellid (
    INPUT = s2cellid_in,
    OUTPUT = s2cellid_out,
    LIKE = int8
);
"#,
    name = "concrete_type",
    creates = [Type(S2CellId)],
    requires = ["shell_type", s2cellid_in, s2cellid_out],
);

extension_sql!(
    r#"
CREATE CAST (s2cellid AS text) WITH FUNCTION s2_cell_to_token(s2cellid);
CREATE CAST (text AS s2cellid) WITH FUNCTION s2_cell_from_token(text);
CREATE CAST (s2cellid AS bigint) WITH FUNCTION s2_cell_to_bigint(s2cellid);
CREATE CAST (bigint AS s2cellid) WITH FUNCTION s2_cell_from_bigint(bigint);
"#,
    name = "s2cellid_casts",
    requires = [
        "concrete_type",
        s2_cell_to_token,
        s2_cell_from_token,
        s2_cell_to_bigint,
        s2_cell_from_bigint,
    ],
);

#[pg_extern]
fn s2_get_extension_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[pg_extern(immutable)]
fn s2_cell_from_token(token: &str) -> S2CellId {
    let cellid = CellID::from_token(token);
    if !s2_cellid_is_valid_raw(cellid.0) {
        error!("invalid s2cellid token");
    }
    S2CellId::from_u64(cellid.0)
}

#[pg_extern(immutable)]
fn s2_cell_to_token(cell: S2CellId) -> String {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    CellID(raw).to_token()
}

#[pg_extern(immutable)]
fn s2_cell_to_bigint(cell: S2CellId) -> i64 {
    cell.value
}

#[pg_extern(immutable)]
fn s2_cell_from_bigint(id: i64) -> S2CellId {
    S2CellId { value: id }
}

#[pg_extern(immutable)]
fn s2_is_valid_cell(cell: S2CellId) -> bool {
    s2_cellid_is_valid_raw(cell.to_u64())
}

#[pg_extern(immutable)]
fn s2_get_level(cell: S2CellId) -> i32 {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    CellID(raw).level() as i32
}

#[pg_extern(immutable)]
fn s2_get_face(cell: S2CellId) -> i32 {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    CellID(raw).face() as i32
}

#[pg_extern(immutable)]
fn s2_lat_lng_to_cell(latlng: Point, level: i32) -> S2CellId {
    if !(0..=30).contains(&level) {
        error!("invalid level");
    }
    let ll = LatLng::from_degrees(latlng.y, latlng.x);
    if !ll.is_valid() {
        error!("invalid latlng");
    }
    let cellid = CellID::from(ll).parent(level as u64);
    S2CellId::from_u64(cellid.0)
}

#[pg_extern(stable, name = "s2_lat_lng_to_cell")]
fn s2_lat_lng_to_cell_default(latlng: Point) -> S2CellId {
    let level = DEFAULT_LEVEL.get();
    s2_lat_lng_to_cell(latlng, level)
}

#[pg_extern(immutable)]
fn s2_cell_to_lat_lng(cell: S2CellId) -> Point {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let ll = LatLng::from(CellID(raw));
    Point {
        x: ll.lng.deg(),
        y: ll.lat.deg(),
    }
}

#[pg_extern(immutable)]
fn s2_cell_to_vertices(cell: S2CellId) -> Vec<Point> {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let verts = Cell::from(CellID(raw)).vertices();
    verts
        .iter()
        .map(|v| {
            let ll = LatLng::from(*v);
            Point {
                x: ll.lng.deg(),
                y: ll.lat.deg(),
            }
        })
        .collect()
}

#[inline]
fn format_polygon_points(points: &[Point]) -> String {
    let mut out = String::new();
    out.push('(');
    for (idx, p) in points.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push('(');
        out.push_str(&format!("{:.15}", p.x));
        out.push(',');
        out.push_str(&format!("{:.15}", p.y));
        out.push(')');
    }
    out.push(')');
    out
}

#[pg_extern(immutable)]
fn s2_cell_boundary_text(cell: S2CellId) -> String {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let verts = Cell::from(CellID(raw)).vertices();
    let points: Vec<Point> = verts
        .iter()
        .map(|v| {
            let ll = LatLng::from(*v);
            Point {
                x: ll.lng.deg(),
                y: ll.lat.deg(),
            }
        })
        .collect();
    format_polygon_points(&points)
}

#[pg_extern(immutable)]
fn s2_cell_edge_neighbors(cell: S2CellId) -> Vec<S2CellId> {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let neighbors = CellID(raw).edge_neighbors();
    neighbors.into_iter().map(|n| S2CellId::from_u64(n.0)).collect()
}

#[pg_extern(immutable)]
fn s2_cell_all_neighbors(cell: S2CellId) -> Vec<S2CellId> {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let cellid = CellID(raw);
    let level = cellid.level();
    cellid
        .all_neighbors(level)
        .into_iter()
        .map(|n| S2CellId::from_u64(n.0))
        .collect()
}

#[pg_extern(stable)]
fn s2_cover_rect(
    rect: pg_sys::BOX,
    level: i32,
    max_cells: i32,
) -> SetOfIterator<'static, S2CellId> {
    if !(0..=30).contains(&level) {
        error!("invalid level");
    }
    if max_cells <= 0 {
        error!("invalid max_cells");
    }
    let lat_lo = rect.low.y.min(rect.high.y);
    let lat_hi = rect.low.y.max(rect.high.y);
    let lng_lo = rect.low.x.min(rect.high.x);
    let lng_hi = rect.low.x.max(rect.high.x);
    let s2_rect = Rect::from_degrees(lat_lo, lng_lo, lat_hi, lng_hi);
    let coverer = RegionCoverer {
        min_level: level as u8,
        max_level: level as u8,
        level_mod: 1,
        max_cells: max_cells as usize,
    };
    let iter = coverer
        .covering(&s2_rect)
        .0
        .into_iter()
        .map(|c| S2CellId::from_u64(c.0));
    SetOfIterator::new(iter)
}

#[pg_extern(stable, name = "s2_cover_rect")]
fn s2_cover_rect_default(rect: pg_sys::BOX) -> SetOfIterator<'static, S2CellId> {
    let level = DEFAULT_COVER_LEVEL.get();
    s2_cover_rect(rect, level, DEFAULT_MAX_CELLS)
}

#[pg_extern(stable)]
fn s2_cover_cap(
    center: Point,
    radius_m: f64,
    level: i32,
    max_cells: i32,
) -> SetOfIterator<'static, S2CellId> {
    if !(0..=30).contains(&level) {
        error!("invalid level");
    }
    if max_cells <= 0 {
        error!("invalid max_cells");
    }
    if radius_m < 0.0 {
        error!("invalid radius");
    }
    let ll = LatLng::from_degrees(center.y, center.x);
    if !ll.is_valid() {
        error!("invalid latlng");
    }
    let center_point = S2Point::from(ll);
    let angle = Angle::from(Rad(radius_m / EARTH_RADIUS_M.get()));
    let cap = Cap::from_center_angle(&center_point, &angle);
    let coverer = RegionCoverer {
        min_level: level as u8,
        max_level: level as u8,
        level_mod: 1,
        max_cells: max_cells as usize,
    };
    let iter = coverer
        .covering(&cap)
        .0
        .into_iter()
        .map(|c| S2CellId::from_u64(c.0));
    SetOfIterator::new(iter)
}

#[pg_extern(stable, name = "s2_cover_cap")]
fn s2_cover_cap_default(center: Point, radius_m: f64) -> SetOfIterator<'static, S2CellId> {
    let level = DEFAULT_COVER_LEVEL.get();
    s2_cover_cap(center, radius_m, level, DEFAULT_MAX_CELLS)
}

extension_sql!(
    r#"
CREATE FUNCTION s2_cover_cap_ranges(
    center point,
    radius_m double precision,
    level integer,
    max_cells integer DEFAULT 8
)
RETURNS SETOF int8range
STABLE PARALLEL SAFE
LANGUAGE SQL
AS $$
    SELECT int8range(
        s2_cell_to_bigint(s2_cell_range_min(cell)),
        s2_cell_to_bigint(s2_cell_range_max(cell)),
        '[]'
    )
    FROM s2_cover_cap($1, $2, $3, $4) AS cell
$$;
"#,
    name = "s2_cover_cap_ranges",
    requires = [s2_cover_cap, s2_cell_range_min, s2_cell_range_max, s2_cell_to_bigint],
);

extension_sql!(
    r#"
CREATE FUNCTION s2_cover_rect_ranges(
    rect box,
    level integer,
    max_cells integer DEFAULT 8
)
RETURNS SETOF int8range
STABLE PARALLEL SAFE
LANGUAGE SQL
AS $$
    SELECT int8range(
        s2_cell_to_bigint(s2_cell_range_min(cell)),
        s2_cell_to_bigint(s2_cell_range_max(cell)),
        '[]'
    )
    FROM s2_cover_rect($1, $2, $3) AS cell
$$;
"#,
    name = "s2_cover_rect_ranges",
    requires = [s2_cover_rect, s2_cell_range_min, s2_cell_range_max, s2_cell_to_bigint],
);

extension_sql!(
    r#"
CREATE FUNCTION s2_cell_to_boundary(cell s2cellid)
RETURNS polygon
IMMUTABLE PARALLEL SAFE
LANGUAGE SQL
AS $$ SELECT s2_cell_boundary_text($1)::polygon $$;
"#,
    name = "s2_cell_to_boundary",
    requires = [s2_cell_boundary_text],
);

#[pg_extern(immutable)]
fn s2_cell_range_min(cell: S2CellId) -> S2CellId {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let min = CellID(raw).range_min();
    S2CellId::from_u64(min.0)
}

#[pg_extern(immutable)]
fn s2_cell_range_max(cell: S2CellId) -> S2CellId {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let max = CellID(raw).range_max();
    S2CellId::from_u64(max.0)
}

#[pg_extern(immutable)]
fn s2_cell_to_parent(cell: S2CellId, level: i32) -> S2CellId {
    if !(0..=30).contains(&level) {
        error!("invalid level");
    }
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let cellid = CellID(raw);
    let cell_level = cellid.level() as i32;
    if level > cell_level {
        error!("invalid level");
    }
    let parent = cellid.parent(level as u64);
    S2CellId::from_u64(parent.0)
}

#[pg_extern(immutable, name = "s2_cell_to_parent")]
fn s2_cell_to_parent_default(cell: S2CellId) -> S2CellId {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let cellid = CellID(raw);
    let level = cellid.level();
    if level == 0 {
        error!("invalid level");
    }
    s2_cell_to_parent(cell, level as i32 - 1)
}

#[pg_extern(immutable)]
fn s2_cell_to_children(cell: S2CellId, level: i32) -> SetOfIterator<'static, S2CellId> {
    if !(0..=30).contains(&level) {
        error!("invalid level");
    }
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let cellid = CellID(raw);
    let cell_level = cellid.level() as i32;
    if level <= cell_level {
        error!("invalid level");
    }
    let mut cur = cellid.child_begin_at_level(level as u64);
    let end = cellid.child_end_at_level(level as u64);
    let iter = std::iter::from_fn(move || {
        if cur == end {
            None
        } else {
            let out = S2CellId::from_u64(cur.0);
            cur = cur.next();
            Some(out)
        }
    });
    SetOfIterator::new(iter)
}

#[pg_extern(immutable, name = "s2_cell_to_children")]
fn s2_cell_to_children_default(cell: S2CellId) -> SetOfIterator<'static, S2CellId> {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let cellid = CellID(raw);
    let level = cellid.level();
    if level == 30 {
        error!("invalid level");
    }
    s2_cell_to_children(cell, level as i32 + 1)
}

#[pg_extern(immutable)]
fn s2_cell_to_center_child(cell: S2CellId, level: i32) -> S2CellId {
    if !(0..=30).contains(&level) {
        error!("invalid level");
    }
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let cellid = CellID(raw);
    let cell_level = cellid.level() as i32;
    if level <= cell_level {
        error!("invalid level");
    }
    let center = Cell::from(cellid).center();
    let child = CellID::from(center).parent(level as u64);
    S2CellId::from_u64(child.0)
}

#[pg_extern(immutable, name = "s2_cell_to_center_child")]
fn s2_cell_to_center_child_default(cell: S2CellId) -> S2CellId {
    let raw = cell.to_u64();
    if !s2_cellid_is_valid_raw(raw) {
        error!("invalid s2cellid");
    }
    let cellid = CellID(raw);
    let level = cellid.level();
    if level == 30 {
        error!("invalid level");
    }
    s2_cell_to_center_child(cell, level as i32 + 1)
}

#[pg_extern(immutable)]
fn s2_great_circle_distance(a: Point, b: Point, unit: &str) -> f64 {
    let ll_a = LatLng::from_degrees(a.y, a.x);
    let ll_b = LatLng::from_degrees(b.y, b.x);
    if !ll_a.is_valid() || !ll_b.is_valid() {
        error!("invalid latlng");
    }
    let angle = ll_a.distance(&ll_b).rad();
    let earth_radius = EARTH_RADIUS_M.get();
    match unit.trim().to_ascii_lowercase().as_str() {
        "m" => angle * earth_radius,
        "km" => angle * earth_radius / 1000.0,
        "rad" => angle,
        _ => error!("invalid unit"),
    }
}

#[pg_extern(immutable, name = "s2_great_circle_distance")]
fn s2_great_circle_distance_default(a: Point, b: Point) -> f64 {
    s2_great_circle_distance(a, b, "m")
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::*;
    use s2::cell::Cell;
    use s2::cellid::CellID;
    use s2::latlng::LatLng;
    use pgrx::spi::Spi;

    #[pg_test]
    fn test_s2_get_extension_version_matches_pkg() {
        let v = s2_get_extension_version();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
    }

    #[pg_test]
    fn test_i64_norm_roundtrip() {
        let cases = [
            0u64,
            1u64,
            0x7fff_ffff_ffff_ffff,
            0x8000_0000_0000_0000,
            u64::MAX,
        ];

        for &v in &cases {
            let norm = u64_to_i64_norm(v);
            let back = i64_norm_to_u64(norm);
            assert_eq!(back, v);
        }
    }

    #[pg_test]
    fn test_i64_norm_order_preserving_unsigned() {
        let pairs = [
            (0u64, 1u64),
            (1u64, 2u64),
            (0x7fff_ffff_ffff_fffe, 0x7fff_ffff_ffff_ffff),
            (0x7fff_ffff_ffff_ffff, 0x8000_0000_0000_0000),
            (0x8000_0000_0000_0000, u64::MAX),
        ];

        for &(a, b) in &pairs {
            let na = u64_to_i64_norm(a);
            let nb = u64_to_i64_norm(b);
            assert!(na < nb, "order violated for {a:#x} < {b:#x}");
        }
    }

    #[pg_test]
    fn test_s2_cell_token_roundtrip() {
        let token = "47a1cbd595522b39";
        let cell = s2_cell_from_token(token);
        let back = s2_cell_to_token(cell);
        assert_eq!(back, token);
    }

    #[pg_test]
    fn test_s2_cell_token_roundtrip_high_bit() {
        let token = "b112966aaaaaaaab";
        let cell = s2_cell_from_token(token);
        let back = s2_cell_to_token(cell);
        assert_eq!(back, token);
    }

    #[pg_test]
    #[should_panic(expected = "invalid s2cellid token")]
    fn test_s2_cell_from_token_invalid() {
        let _ = s2_cell_from_token("zz");
    }

    #[pg_test]
    fn test_s2_cell_to_bigint() {
        let token = "47a1cbd595522b39";
        let cell = s2_cell_from_token(token);
        let expected = u64_to_i64_norm(CellID::from_token(token).0);
        assert_eq!(s2_cell_to_bigint(cell), expected);
    }

    #[pg_test]
    fn test_s2_cell_from_bigint_roundtrip() {
        let token = "47a1cbd595522b39";
        let cell = s2_cell_from_token(token);
        let id = s2_cell_to_bigint(cell);
        let back = s2_cell_from_bigint(id);
        assert_eq!(s2_cell_to_token(back), token);
    }

    #[pg_test]
    fn test_s2_is_valid_cell() {
        let valid = s2_cell_from_token("47a1cbd595522b39");
        assert!(s2_is_valid_cell(valid));

        let valid_high_bit = s2_cell_from_token("b112966aaaaaaaab");
        assert!(s2_is_valid_cell(valid_high_bit));

        let invalid = s2_cell_from_bigint(0);
        assert!(!s2_is_valid_cell(invalid));
    }

    #[pg_test]
    fn test_s2_get_level_and_face() {
        let face0 = s2_cell_from_token("1");
        assert_eq!(s2_get_level(face0), 0);
        assert_eq!(s2_get_face(face0), 0);

        let face1 = s2_cell_from_token("3");
        assert_eq!(s2_get_level(face1), 0);
        assert_eq!(s2_get_face(face1), 1);
    }

    #[pg_test]
    fn test_s2_lat_lng_to_cell_level() {
        let lat = 49.703498679;
        let lng = 11.770681595;
        let level = 12i32;
        let ll = LatLng::from_degrees(lat, lng);
        let expected = CellID::from(ll).parent(level as u64).to_token();

        let got = s2_lat_lng_to_cell(Point { x: lng, y: lat }, level);
        assert_eq!(s2_cell_to_token(got), expected);
    }

    #[pg_test]
    #[should_panic(expected = "invalid level")]
    fn test_s2_lat_lng_to_cell_level_invalid() {
        let _ = s2_lat_lng_to_cell(Point { x: 0.0, y: 0.0 }, 31);
    }

    #[pg_test]
    fn test_s2_lat_lng_to_cell_default_level() {
        let lat = 49.703498679;
        let lng = 11.770681595;
        let expected = s2_lat_lng_to_cell(Point { x: lng, y: lat }, 14);

        let got = s2_lat_lng_to_cell_default(Point { x: lng, y: lat });
        assert_eq!(s2_cell_to_token(got), s2_cell_to_token(expected));
    }

    #[pg_test]
    fn test_s2_lat_lng_to_cell_default_level_guc() {
        let lat = 49.703498679;
        let lng = 11.770681595;
        Spi::run("SET pg_s2.default_level = 10").expect("set GUC");

        let expected = s2_lat_lng_to_cell(Point { x: lng, y: lat }, 10);
        let got = s2_lat_lng_to_cell_default(Point { x: lng, y: lat });
        assert_eq!(s2_cell_to_token(got), s2_cell_to_token(expected));
    }

    #[pg_test]
    fn test_s2_cell_to_lat_lng() {
        let cell = s2_cell_from_token("47a1cbd595522b39");
        let ll = s2_cell_to_lat_lng(cell);
        assert!((ll.y - 49.703498679).abs() < 1e-6);
        assert!((ll.x - 11.770681595).abs() < 1e-6);
    }

    #[pg_test]
    #[should_panic(expected = "invalid s2cellid")]
    fn test_s2_cell_to_lat_lng_invalid() {
        let _ = s2_cell_to_lat_lng(s2_cell_from_bigint(0));
    }

    #[pg_test]
    fn test_s2_cell_range_min() {
        let token = "47a1cbd595522b39";
        let cell = s2_cell_from_token(token);
        let expected = CellID::from_token(token).range_min().to_token();
        let got = s2_cell_range_min(cell);
        assert_eq!(s2_cell_to_token(got), expected);
    }

    #[pg_test]
    fn test_s2_cell_range_max() {
        let token = "47a1cbd595522b39";
        let cell = s2_cell_from_token(token);
        let expected = CellID::from_token(token).range_max().to_token();
        let got = s2_cell_range_max(cell);
        assert_eq!(s2_cell_to_token(got), expected);
    }

    #[pg_test]
    fn test_s2_cell_to_parent_level() {
        let token = "47a1cbd595522b39";
        let cell_raw = CellID::from_token(token);
        assert!(cell_raw.level() > 0);
        let level = cell_raw.level() as i32 - 1;
        let expected = cell_raw.parent(level as u64).to_token();
        let cell = s2_cell_from_token(token);
        let got = s2_cell_to_parent(cell, level);
        assert_eq!(s2_cell_to_token(got), expected);
    }

    #[pg_test]
    fn test_s2_cell_to_parent_default() {
        let token = "47a1cbd595522b39";
        let cell_raw = CellID::from_token(token);
        assert!(cell_raw.level() > 0);
        let expected = cell_raw.parent(cell_raw.level() - 1).to_token();
        let cell = s2_cell_from_token(token);
        let got = s2_cell_to_parent_default(cell);
        assert_eq!(s2_cell_to_token(got), expected);
    }

    #[pg_test]
    fn test_s2_cell_to_children_level() {
        let ll = LatLng::from_degrees(49.703498679, 11.770681595);
        let cell_raw = CellID::from(ll).parent(10);
        let level = cell_raw.level() as i32 + 1;
        let expected: Vec<String> = cell_raw.children().iter().map(|c| c.to_token()).collect();
        let cell = s2_cell_from_token(&cell_raw.to_token());
        let got: Vec<String> = s2_cell_to_children(cell, level)
            .map(s2_cell_to_token)
            .collect();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cell_to_children_default() {
        let ll = LatLng::from_degrees(49.703498679, 11.770681595);
        let cell_raw = CellID::from(ll).parent(10);
        let expected: Vec<String> = cell_raw.children().iter().map(|c| c.to_token()).collect();
        let cell = s2_cell_from_token(&cell_raw.to_token());
        let got: Vec<String> = s2_cell_to_children_default(cell)
            .map(s2_cell_to_token)
            .collect();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cell_to_center_child_level() {
        let ll = LatLng::from_degrees(49.703498679, 11.770681595);
        let cell_raw = CellID::from(ll).parent(10);
        let level = cell_raw.level() as i32 + 1;
        let expected = CellID::from(Cell::from(cell_raw).center())
            .parent(level as u64)
            .to_token();
        let cell = s2_cell_from_token(&cell_raw.to_token());
        let got = s2_cell_to_center_child(cell, level);
        assert_eq!(s2_cell_to_token(got), expected);
    }

    #[pg_test]
    fn test_s2_cell_to_center_child_default() {
        let ll = LatLng::from_degrees(49.703498679, 11.770681595);
        let cell_raw = CellID::from(ll).parent(10);
        let level = cell_raw.level() as i32 + 1;
        let expected = CellID::from(Cell::from(cell_raw).center())
            .parent(level as u64)
            .to_token();
        let cell = s2_cell_from_token(&cell_raw.to_token());
        let got = s2_cell_to_center_child_default(cell);
        assert_eq!(s2_cell_to_token(got), expected);
    }

    #[pg_test]
    fn test_s2_cell_to_vertices() {
        let token = "47a1cbd595522b39";
        let cell_raw = CellID::from_token(token);
        let expected: Vec<Point> = Cell::from(cell_raw)
            .vertices()
            .iter()
            .map(|v| {
                let ll = LatLng::from(*v);
                Point {
                    x: ll.lng.deg(),
                    y: ll.lat.deg(),
                }
            })
            .collect();
        let cell = s2_cell_from_token(token);
        let got = s2_cell_to_vertices(cell);
        assert_eq!(got.len(), expected.len());
        for (a, b) in got.iter().zip(expected.iter()) {
            assert!((a.x - b.x).abs() < 1e-6);
            assert!((a.y - b.y).abs() < 1e-6);
        }
    }

    #[pg_test]
    fn test_s2_cell_boundary_text() {
        let token = "47a1cbd595522b39";
        let cell_raw = CellID::from_token(token);
        let points: Vec<Point> = Cell::from(cell_raw)
            .vertices()
            .iter()
            .map(|v| {
                let ll = LatLng::from(*v);
                Point {
                    x: ll.lng.deg(),
                    y: ll.lat.deg(),
                }
            })
            .collect();
        let expected = format_polygon_points(&points);
        let cell = s2_cell_from_token(token);
        let got = s2_cell_boundary_text(cell);
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cell_to_boundary_sql() {
        let token = "47a1cbd595522b39";
        let query = format!(
            "SELECT s2_cell_to_boundary(s2_cell_from_token('{token}')) IS NOT NULL"
        );
        let got = Spi::get_one::<bool>(&query).expect("spi");
        assert_eq!(got, Some(true));
    }

    #[pg_test]
    fn test_s2_cover_rect_level() {
        let rect = pg_sys::BOX {
            low: Point { x: 11.70, y: 49.68 },
            high: Point { x: 11.82, y: 49.76 },
        };
        let level = 12i32;
        let max_cells = 8i32;
        let s2_rect = s2::rect::Rect::from_degrees(
            rect.low.y,
            rect.low.x,
            rect.high.y,
            rect.high.x,
        );
        let coverer = s2::region::RegionCoverer {
            min_level: level as u8,
            max_level: level as u8,
            level_mod: 1,
            max_cells: max_cells as usize,
        };
        let mut expected: Vec<String> =
            coverer.covering(&s2_rect).0.iter().map(|c| c.to_token()).collect();
        expected.sort();
        let mut got: Vec<String> = s2_cover_rect(rect, level, max_cells)
            .map(s2_cell_to_token)
            .collect();
        got.sort();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cover_rect_default_level() {
        let rect = pg_sys::BOX {
            low: Point { x: 11.70, y: 49.68 },
            high: Point { x: 11.82, y: 49.76 },
        };
        let expected: Vec<String> = s2_cover_rect(rect, 12, 8).map(s2_cell_to_token).collect();
        let got: Vec<String> = s2_cover_rect_default(rect).map(s2_cell_to_token).collect();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cover_rect_default_level_guc() {
        let rect = pg_sys::BOX {
            low: Point { x: 11.70, y: 49.68 },
            high: Point { x: 11.82, y: 49.76 },
        };
        Spi::run("SET pg_s2.default_cover_level = 10").expect("set GUC");
        let expected: Vec<String> = s2_cover_rect(rect, 10, 8).map(s2_cell_to_token).collect();
        let got: Vec<String> = s2_cover_rect_default(rect).map(s2_cell_to_token).collect();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cover_cap_level() {
        let center = Point { x: 11.77, y: 49.70 };
        let radius_m = 2000.0;
        let level = 12i32;
        let max_cells = 8i32;
        let center_ll = LatLng::from_degrees(center.y, center.x);
        let center_point = s2::point::Point::from(center_ll);
        let angle = s2::s1::Angle::from(s2::s1::Rad(radius_m / EARTH_RADIUS_M.get()));
        let cap = s2::cap::Cap::from_center_angle(&center_point, &angle);
        let coverer = s2::region::RegionCoverer {
            min_level: level as u8,
            max_level: level as u8,
            level_mod: 1,
            max_cells: max_cells as usize,
        };
        let mut expected: Vec<String> =
            coverer.covering(&cap).0.iter().map(|c| c.to_token()).collect();
        expected.sort();
        let mut got: Vec<String> = s2_cover_cap(center, radius_m, level, max_cells)
            .map(s2_cell_to_token)
            .collect();
        got.sort();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cover_cap_default_level() {
        let center = Point { x: 11.77, y: 49.70 };
        let radius_m = 2000.0;
        let expected: Vec<String> = s2_cover_cap(center, radius_m, 12, 8)
            .map(s2_cell_to_token)
            .collect();
        let got: Vec<String> = s2_cover_cap_default(center, radius_m)
            .map(s2_cell_to_token)
            .collect();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cover_cap_ranges_sql() {
        let center = Point { x: 11.77, y: 49.70 };
        let radius_m = 2000.0;
        let level = 12i32;
        let max_cells = 8i32;
        let center_ll = LatLng::from_degrees(center.y, center.x);
        let center_point = s2::point::Point::from(center_ll);
        let angle = s2::s1::Angle::from(s2::s1::Rad(radius_m / EARTH_RADIUS_M.get()));
        let cap = s2::cap::Cap::from_center_angle(&center_point, &angle);
        let coverer = s2::region::RegionCoverer {
            min_level: level as u8,
            max_level: level as u8,
            level_mod: 1,
            max_cells: max_cells as usize,
        };
        let mut expected: Vec<String> = coverer
            .covering(&cap)
            .0
            .iter()
            .map(|c| {
                let min = u64_to_i64_norm(c.range_min().0);
                let max = u64_to_i64_norm(c.range_max().0);
                let max_exclusive = max.saturating_add(1);
                format!("[{min},{max_exclusive})")
            })
            .collect();
        expected.sort();
        let query = format!(
            "SELECT string_agg(r::text, ',' ORDER BY r::text) \
             FROM s2_cover_cap_ranges(point({}, {}), {}::double precision, {}, {}) r",
            center.x, center.y, radius_m, level, max_cells
        );
        let got = Spi::get_one::<String>(&query).expect("spi");
        let got_list = got.unwrap_or_default();
        let expected_list = expected.join(",");
        assert_eq!(got_list, expected_list);
    }

    #[pg_test]
    fn test_s2_cover_rect_ranges_sql() {
        let rect = pg_sys::BOX {
            low: Point { x: 11.70, y: 49.68 },
            high: Point { x: 11.82, y: 49.76 },
        };
        let level = 12i32;
        let max_cells = 8i32;
        let s2_rect = s2::rect::Rect::from_degrees(
            rect.low.y,
            rect.low.x,
            rect.high.y,
            rect.high.x,
        );
        let coverer = s2::region::RegionCoverer {
            min_level: level as u8,
            max_level: level as u8,
            level_mod: 1,
            max_cells: max_cells as usize,
        };
        let mut expected: Vec<String> = coverer
            .covering(&s2_rect)
            .0
            .iter()
            .map(|c| {
                let min = u64_to_i64_norm(c.range_min().0);
                let max = u64_to_i64_norm(c.range_max().0);
                let max_exclusive = max.saturating_add(1);
                format!("[{min},{max_exclusive})")
            })
            .collect();
        expected.sort();
        let query = format!(
            "SELECT string_agg(r::text, ',' ORDER BY r::text) \
             FROM s2_cover_rect_ranges(box(point({}, {}), point({}, {})), {}, {}) r",
            rect.low.x, rect.low.y, rect.high.x, rect.high.y, level, max_cells
        );
        let got = Spi::get_one::<String>(&query).expect("spi");
        let got_list = got.unwrap_or_default();
        let expected_list = expected.join(",");
        assert_eq!(got_list, expected_list);
    }

    #[pg_test]
    fn test_s2_great_circle_distance_units() {
        let a = Point { x: 0.0, y: 0.0 };
        let b = Point { x: 90.0, y: 0.0 };
        let ll_a = LatLng::from_degrees(a.y, a.x);
        let ll_b = LatLng::from_degrees(b.y, b.x);
        let angle = ll_a.distance(&ll_b).rad();
        let earth_radius = EARTH_RADIUS_M.get();
        let expected_m = angle * earth_radius;
        let expected_km = expected_m / 1000.0;

        let got_m = s2_great_circle_distance(a, b, "m");
        let got_km = s2_great_circle_distance(a, b, "km");
        let got_default = s2_great_circle_distance_default(a, b);
        let got_rad = s2_great_circle_distance(a, b, "rad");

        assert!((got_m - expected_m).abs() < 1e-6);
        assert!((got_km - expected_km).abs() < 1e-9);
        assert!((got_default - expected_m).abs() < 1e-6);
        assert!((got_rad - angle).abs() < 1e-12);
    }

    #[pg_test]
    fn test_s2_great_circle_distance_guc() {
        let a = Point { x: 0.0, y: 0.0 };
        let b = Point { x: 90.0, y: 0.0 };
        let ll_a = LatLng::from_degrees(a.y, a.x);
        let ll_b = LatLng::from_degrees(b.y, b.x);
        let angle = ll_a.distance(&ll_b).rad();
        Spi::run("SET pg_s2.earth_radius_m = 1000000").expect("set GUC");
        let got = s2_great_circle_distance(a, b, "m");
        let expected = angle * 1_000_000.0;
        assert!((got - expected).abs() < 1e-6);
    }

    #[pg_test]
    fn test_s2_cellid_casts() {
        let token = "47a1cbd595522b39";
        let cast_token = Spi::get_one::<String>(&format!(
            "SELECT ('{token}'::text::s2cellid)::text"
        ))
        .expect("spi");
        assert_eq!(cast_token, Some(token.to_string()));

        let cell = s2_cell_from_token(token);
        let expected_bigint = s2_cell_to_bigint(cell);
        let cast_bigint = Spi::get_one::<i64>(&format!(
            "SELECT (s2_cell_from_token('{token}')::bigint)"
        ))
        .expect("spi");
        assert_eq!(cast_bigint, Some(expected_bigint));

        let cast_back = Spi::get_one::<String>(&format!(
            "SELECT (CAST({expected_bigint} AS bigint)::s2cellid)::text"
        ))
        .expect("spi");
        assert_eq!(cast_back, Some(token.to_string()));
    }

    #[pg_test]
    fn test_s2_cell_edge_neighbors() {
        let token = "47a1cbd595522b39";
        let cell_raw = CellID::from_token(token);
        let mut expected: Vec<String> = cell_raw
            .edge_neighbors()
            .iter()
            .map(|c| c.to_token())
            .collect();
        expected.sort();
        let cell = s2_cell_from_token(token);
        let mut got: Vec<String> = s2_cell_edge_neighbors(cell)
            .iter()
            .map(|c| s2_cell_to_token(*c))
            .collect();
        got.sort();
        assert_eq!(got, expected);
    }

    #[pg_test]
    fn test_s2_cell_all_neighbors() {
        let token = "47a1cbd595522b39";
        let cell_raw = CellID::from_token(token);
        let mut expected: Vec<String> = cell_raw
            .all_neighbors(cell_raw.level())
            .iter()
            .map(|c| c.to_token())
            .collect();
        expected.sort();
        let cell = s2_cell_from_token(token);
        let mut got: Vec<String> = s2_cell_all_neighbors(cell)
            .iter()
            .map(|c| s2_cell_to_token(*c))
            .collect();
        got.sort();
        assert_eq!(got, expected);
    }
}

/// This module is required by `cargo pgrx test` invocations.
/// It must be visible at the root of your extension crate.
#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}

    #[must_use]
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
