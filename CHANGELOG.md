# Changelog

All notable changes to this project will be documented in this file.

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
