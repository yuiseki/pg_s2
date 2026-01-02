use pgrx::callconv::{ArgAbi, BoxRet};
use pgrx::datum::Datum;
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

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::*;
    use s2::latlng::LatLng;

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
