use pgrx::callconv::{ArgAbi, BoxRet};
use pgrx::datum::Datum;
use pgrx::guc::{GucContext, GucFlags, GucRegistry, GucSetting};
use pgrx::pg_sys::Point;
use pgrx::pg_sys::Oid;
use pgrx::pgrx_sql_entity_graph::metadata::{
    ArgumentError, Returns, ReturnsError, SqlMapping, SqlTranslatable,
};
use pgrx::prelude::*;
use pgrx::{rust_regtypein, StringInfo};
use s2::cellid::{CellID, NUM_FACES, POS_BITS};
use s2::latlng::LatLng;
use std::ffi::CStr;

::pgrx::pg_module_magic!(name, version);

const S2CELLID_ORDER_MASK: u64 = 0x8000_0000_0000_0000;
const S2CELLID_LSB_MASK: u64 = 0x1555_5555_5555_5555;
static DEFAULT_LEVEL: GucSetting<i32> = GucSetting::<i32>::new(14);
static DEFAULT_LEVEL_NAME: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"pg_s2.default_level\0") };
static DEFAULT_LEVEL_SHORT: &CStr = unsafe {
    CStr::from_bytes_with_nul_unchecked(b"Default S2 level for s2_lat_lng_to_cell(point).\0")
};
static DEFAULT_LEVEL_DESC: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"Used when level is not explicitly provided.\0") };

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

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::*;
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
