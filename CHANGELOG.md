# Changelog

All notable changes to this project will be documented in this file.

## v0.0.5

- Add PG14 to CI matrix and declare PG14–17 support
- PGXN metadata updates (META.json tags/release status, provides file path)
- Add local `make install` and PGXN zip packaging targets

## v0.0.1

- Initial MVP-0 release
- Added `s2cellid` type (order-preserving int8-like storage)
- Token conversion functions
- Bigint conversion functions
- Validation and metadata functions
- Lat/Lng to CellID and CellID to Lat/Lng
- Parent/children/center-child hierarchy functions
- Range min/max helpers
- Cell vertices function
- GUC: `pg_s2.default_level`
- Docker-based build and test via `make test`

## v0.0.2

- Added casts between `s2cellid` and `text`/`bigint`
- Added `s2_cell_to_boundary` (polygon via text cast) and `s2_cell_bbox`
- Added cap and rect covering APIs plus range helpers
- Added great-circle distance with configurable earth radius
- Added GUCs: `pg_s2.default_cover_level`, `pg_s2.earth_radius_m`
- Docker build caching improvement for faster tests
