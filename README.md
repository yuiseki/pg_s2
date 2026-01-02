# pg_s2

PostgreSQL extension that exposes a minimal S2 CellID API for indexing and basic spatial workflows.
This is an early, MVP-oriented release focused on correctness and testability.

## Status

- Version: v0.0.1
- Scope: MVP-0 subset of SPEC.md

## Features

- `s2cellid` type (int8-like, order-preserving)
- Token conversion: `s2_cell_to_token`, `s2_cell_from_token`
- Bigint conversion: `s2_cell_to_bigint`, `s2_cell_from_bigint`
- Validation and metadata: `s2_is_valid_cell`, `s2_get_level`, `s2_get_face`
- Lat/Lng conversion: `s2_lat_lng_to_cell`, `s2_cell_to_lat_lng`
- Hierarchy: `s2_cell_to_parent`, `s2_cell_to_children`, `s2_cell_to_center_child`
- Range helpers: `s2_cell_range_min`, `s2_cell_range_max`
- Vertices: `s2_cell_to_vertices`
- GUC: `pg_s2.default_level`

## Requirements

- PostgreSQL 17
- Rust toolchain (handled in Docker build)
- Docker + docker compose

## Build and Test (Docker)

```
make test
```

This builds the extension in a container and runs `cargo pgrx test` inside Docker.

## Usage Examples

```sql
CREATE EXTENSION pg_s2;

-- Convert lat/lng to cell (level 14)
SELECT s2_lat_lng_to_cell(point(139.767, 35.681), 14);

-- Default level via GUC
SET pg_s2.default_level = 12;
SELECT s2_lat_lng_to_cell(point(139.767, 35.681));

-- Token roundtrip
SELECT s2_cell_to_token(s2_cell_from_token('47a1cbd595522b39'));

-- Parent/children
SELECT s2_cell_to_parent(s2_cell_from_token('47a1cbd595522b39'));
SELECT * FROM s2_cell_to_children(s2_cell_from_token('47a1cbd595522b39'));

-- Range helpers for B-tree filtering
SELECT s2_cell_range_min(s2_cell_from_token('47a1cbd595522b39')),
       s2_cell_range_max(s2_cell_from_token('47a1cbd595522b39'));
```

## Development Notes

- Specs live in `SPEC.md`.
- Tests are in `src/lib.rs` and run in Docker.

## License

TBD
