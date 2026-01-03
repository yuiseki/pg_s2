# pg_s2

PostgreSQL extension that exposes a minimal S2 CellID API for indexing and basic spatial workflows.
This is an early, MVP-oriented release focused on correctness and testability.

## Status

- Version: v0.0.5
- Scope: MVP-0 subset of SPEC.md (expanded)

## Features

- `s2cellid` type (int8-like, order-preserving)
- Token conversion: `s2_cell_to_token`, `s2_cell_from_token`
- Bigint conversion: `s2_cell_to_bigint`, `s2_cell_from_bigint`
- Casts: `s2cellid` ↔ `text`, `s2cellid` ↔ `bigint`
- Validation and metadata: `s2_is_valid_cell`, `s2_get_level`, `s2_get_face`
- Lat/Lng conversion: `s2_lat_lng_to_cell`, `s2_cell_to_lat_lng`
- Hierarchy: `s2_cell_to_parent`, `s2_cell_to_children`, `s2_cell_to_center_child`
- Range helpers: `s2_cell_range_min`, `s2_cell_range_max`
- Boundary and bbox: `s2_cell_to_boundary`, `s2_cell_to_vertices`, `s2_cell_bbox`
- Covering: `s2_cover_cap`, `s2_cover_rect`, `s2_cover_cap_ranges`, `s2_cover_rect_ranges`
- Distance: `s2_great_circle_distance`
- GUCs: `pg_s2.default_level`, `pg_s2.default_cover_level`, `pg_s2.earth_radius_m`

## SPEC.md v0.1 MVP coverage

This implementation targets **SPEC.md v0.1 MVP**. Mapping summary:

- §7 Extension: `s2_get_extension_version`
- §8 Indexing: `s2_lat_lng_to_cell`, `s2_cell_to_lat_lng`, `s2_cell_to_boundary`, `s2_cell_to_vertices`
- §9 Inspection: `s2_is_valid_cell`, `s2_get_level`, `s2_get_face`, token/bigint conversions
- §10 Hierarchy: parent/children/center_child, range min/max
- §11 Traversal: edge/all neighbors
- §12 Region: cap/rect covering + ranges
- §13 Misc: `s2_great_circle_distance`
- §14 Casts/operators/opclass: `s2cellid` casts, comparison ops, B-tree opclass
- §15 GUCs: `pg_s2.default_level`, `pg_s2.default_cover_level`, `pg_s2.earth_radius_m`

Notes:
- `s2_cell_bbox` is implemented but not part of SPEC.md v0.1 MVP.

## Requirements

- PostgreSQL 14–17
- Rust toolchain (handled in Docker build)
- Docker + docker compose

## Build and Test (Docker)

```
make test
```

This builds the extension in a container and runs `cargo pgrx test` inside Docker.

## Package (Docker)

```
make package
```

Artifacts are collected under `build/pg17` by default. To target a different
PostgreSQL version:

```
PG_MAJOR=16 make package
```

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

-- Bounding box for a cell
SELECT s2_cell_bbox(s2_cell_from_token('47a1cbd595522b39'));

-- Cover a cap and get ranges for prefiltering
SELECT * FROM s2_cover_cap_ranges(point(139.767, 35.681), 2000.0, 12, 16);

-- Great-circle distance in meters
SELECT s2_great_circle_distance(point(139.767, 35.681), point(135.502, 34.693));
```

## B-tree Index Pattern (Recommended)

### Quick Start

```sql
-- 1. Create table with s2cellid column
CREATE TABLE locations (
    id SERIAL PRIMARY KEY,
    name TEXT,
    latlng POINT,
    cell s2cellid
);

-- 2. Create B-tree index (ORDER-PRESERVING)
CREATE INDEX idx_locations_cell ON locations USING btree(cell);

-- 3. Populate cells
UPDATE locations SET cell = s2_lat_lng_to_cell(latlng, 14);
```

### Two-Stage Filtering Pattern

**Recommended Pattern**: Coarse filter with B-tree → Precise validation

```sql
-- Example: Find locations within 2km of Tokyo Station
WITH target AS (
    SELECT point(139.767, 35.681) AS pt
),
cover AS (
    SELECT range_min, range_max
    FROM target, LATERAL s2_cover_cap_ranges(pt, 2000.0, 12, 16)
)
SELECT l.*
FROM locations l, target t, cover c
WHERE l.cell BETWEEN c.range_min AND c.range_max  -- Stage 1: B-tree range scan
  AND s2_great_circle_distance(l.latlng, t.pt) <= 2000.0;  -- Stage 2: precise check
```

### Why BETWEEN Works

- `s2cellid` uses **order-preserving encoding** (`i64_norm`)
- Range filters map to contiguous B-tree leaf scans
- Example:
  ```sql
  -- Parent cell: 47a1cbd595522b39
  SELECT s2_cell_range_min(s2_cell_from_token('47a1cbd595522b39')) AS rmin,
         s2_cell_range_max(s2_cell_from_token('47a1cbd595522b39')) AS rmax;
  
  -- All descendants fit in [rmin, rmax]
  SELECT * FROM locations WHERE cell BETWEEN rmin AND rmax;
  ```

### EXPLAIN Example

```sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT *
FROM locations
WHERE cell BETWEEN s2_cell_range_min(s2_cell_from_token('47a1cbd595522b39'))
                AND s2_cell_range_max(s2_cell_from_token('47a1cbd595522b39'));

-- Expected plan:
--  Index Scan using idx_locations_cell on locations
--    Index Cond: (cell >= rmin AND cell <= rmax)
--    Buffers: shared hit=X
```

### Best Practices

1. **Always use B-tree index** on `s2cellid` columns
2. **Two-stage filter** for spatial queries:
   - Stage 1: `WHERE cell BETWEEN range_min AND range_max` (fast, B-tree)
   - Stage 2: Precise distance/containment check (accurate)
3. **Choose covering levels wisely**:
   - `min_level`: Lower = fewer ranges, more false positives
   - `max_level`: Higher = more ranges, tighter fit
   - Typical: `min=12, max=16` for city-scale queries
4. **Monitor EXPLAIN**: Ensure "Index Scan" (not Seq Scan)

### Common Patterns

```sql
-- Pattern 1: Radius search (cap)
WITH cover AS (
    SELECT * FROM s2_cover_cap_ranges(:center, :radius_m, 12, 16)
)
SELECT l.*
FROM locations l, cover c
WHERE l.cell BETWEEN c.range_min AND c.range_max
  AND s2_great_circle_distance(l.latlng, :center) <= :radius_m;

-- Pattern 2: Bounding box search (rect)
WITH cover AS (
    SELECT * FROM s2_cover_rect_ranges(:sw_corner, :ne_corner, 12, 16)
)
SELECT l.*
FROM locations l, cover c
WHERE l.cell BETWEEN c.range_min AND c.range_max
  AND l.latlng <@ box(:sw_corner, :ne_corner);  -- precise bbox check

-- Pattern 3: Find neighbors (parent cell)
WITH target_cell AS (
    SELECT s2_lat_lng_to_cell(:point, 14) AS cell
),
parent AS (
    SELECT s2_cell_to_parent(cell) AS pcell FROM target_cell
)
SELECT l.*
FROM locations l, parent p
WHERE l.cell BETWEEN s2_cell_range_min(p.pcell)
                  AND s2_cell_range_max(p.pcell);
```

## Development Notes

- Specs live in `SPEC.md`.
- Tests are in `src/lib.rs` and run in Docker.

## License

TBD
