# pg_s2 — SPEC

Status: Draft  
Target: PostgreSQL 14–17 (primary), 13+ (best-effort)  
Implementation: pgrx + Rust S2 (yjh0502/rust-s2 / crate `s2`)

---

## 1. 概要

`pg_s2` は PostgreSQL 上で **S2 CellID** を第一級オブジェクトとして扱えるようにする拡張です。

- **Point（緯度経度）→ S2 CellID** を高速に変換
- **S2 CellID → center / boundary** を取得
- **階層（parent/children）**、**近傍（neighbors）**、**region covering（cap/rect）** を提供
- PostGIS 依存なし（`point` / `polygon` / `box` 等の組み込み型を使用）

目的は「PostgreSQL で S2 Geometry が使えて、精度と速度が向上して嬉しい」を **最短で体感** できる範囲をカバーすること。

---

## 2. 目的 / 非目的

### 2.1 目的 (Goals)

- S2 CellID を **8 バイト固定長** として保持し、比較・ソート・B-tree インデックスで扱えること
- h3-pg に近い使い心地の API（命名・カテゴリ分け・戻り値の型）
- 代表的なユースケースをカバー
  - 点データのグリッド化・クラスタリング
  - radius / bbox 検索の事前フィルタ（covering + range）
  - セル階層での集計（parent へのロールアップ）
- PostgreSQL 17 でビルド・動作すること

### 2.2 非目的 (Non-goals)

- pgs2 / s2-postgis との互換性（関数名、型名、振る舞い）の維持
- PostGIS との統合（geometry/geography 型の入出力、ST\_\* 連携）
- S2Polygon / S2Loop / S2Polyline の完全実装
- 専用インデックス（SP-GiST / GIN など）の初期リリースでの提供  
  ※ v0.1 は B-tree を中心に設計する

---

## 3. 用語

- **latlng**: 緯度経度（度）。PostgreSQL の `point` を使用。
  - `point.x = longitude (deg)`
  - `point.y = latitude (deg)`
- **level**: S2 の階層レベル。`0..=30`（S2 の最大レベルは 30）
- **token**: S2 CellID の hex 表現（末尾の 0 を省略した短縮表現）

---

## 4. 依存関係

- Rust: `pgrx`
- Rust S2: `yjh0502/rust-s2`（crate 名は `s2` を想定）
- PostgreSQL 組み込み型のみ使用（PostGIS 不要）

---

## 5. データ型

### 5.1 `s2cellid`

#### 5.1.1 物理表現（重要）

S2 CellID は本来 `u64` だが PostgreSQL は `u64` を持たない。

`pg_s2` は **B-tree による範囲検索** を壊さないため、内部表現として **order-preserving な int8** を採用する。

- `u64` の自然順序（unsigned order）を保ちたい
- そのまま `i64` にキャストすると符号境界で順序が壊れる
- よって内部表現 `i64_norm` を以下で定義する:

```
i64_norm = (u64_cellid ^ 0x8000_0000_0000_0000) as i64
u64_cellid = (i64_norm as u64) ^ 0x8000_0000_0000_0000
```

この `i64_norm` 同士を signed 比較すると **unsigned の順序と一致** する。

> NOTE: BigQuery 等でも「符号付き int64 にビット等価で格納し負値になり得る」アプローチがあるが、pg_s2 はさらに “順序保証” を重視し、xor により比較順序を安定化させる。

#### 5.1.2 SQL リテラル（入出力）

- `s2cellid` の SQL 表現は **token（text）** を標準とする
- `::text` で token を得る
- `text::s2cellid` で token を解釈して得る
- `bigint` との相互変換も提供（ただし bigint は内部の `i64_norm`）

#### 5.1.3 NULL / エラー

- 入力が不正（token が不正、level 範囲外、lat/lng が範囲外等）の場合は ERROR
- `s2_try_*` 系（戻り値 nullable）は v0.2+ で検討（v0.1 は必須ではない）

---

## 6. API 設計方針

- 関数名は `s2_` プレフィックス + snake_case
- h3-pg のカテゴリに寄せる
  - Indexing
  - Inspection
  - Hierarchical
  - Traversal
  - Region
  - Misc
- “便利関数” は増やしすぎない。**最短で体感** できる範囲に絞る。

---

## 7. SQL API（v0.1 MVP）

以下は **v0.1 で実装 MUST** の範囲。

### 7.1 Extension

#### 7.1.1 バージョン

- `s2_get_extension_version() -> text`
  - 拡張のバージョン（例: `0.1.0`）
  - VOLATILE（ビルド時に固定でもよい）

---

## 8. Indexing functions

### 8.1 `s2_lat_lng_to_cell`

- `s2_lat_lng_to_cell(latlng point, level integer) -> s2cellid`
  - 入力 `latlng` の点を包含する CellID（指定 level）を返す
  - `level` は `0..=30` のみ許可
  - IMMUTABLE
- `s2_lat_lng_to_cell(latlng point) -> s2cellid`
  - デフォルト level を用いる（設定 `pg_s2.default_level`）
  - STABLE（GUC 参照のため）

### 8.2 `s2_cell_to_lat_lng`

- `s2_cell_to_lat_lng(cell s2cellid) -> point`
  - セルの重心（centroid）を返す
  - IMMUTABLE

### 8.3 `s2_cell_to_boundary`

- `s2_cell_to_boundary(cell s2cellid) -> polygon`
  - セル境界を `polygon`（頂点配列）として返す
  - 頂点は `point(x=lng, y=lat)` の順
  - 返却 polygon は「閉曲線」扱い（PostgreSQL の polygon 仕様に従う）
  - antimeridian（±180° 跨ぎ）の “extend” は v0.2+（MAY）
  - IMMUTABLE

### 8.4 `s2_cell_to_vertices`

- `s2_cell_to_vertices(cell s2cellid) -> point[]`
  - 境界頂点を配列で返す（4 頂点）
  - IMMUTABLE

---

## 9. Index inspection functions

### 9.1 Validity / metadata

- `s2_is_valid_cell(cell s2cellid) -> boolean`
  - IMMUTABLE
- `s2_get_level(cell s2cellid) -> integer`
  - IMMUTABLE
- `s2_get_face(cell s2cellid) -> integer`
  - 0..=5 の face index
  - IMMUTABLE

### 9.2 Token conversion

- `s2_cell_to_token(cell s2cellid) -> text`
  - token は hex、末尾の 0 を省略
  - IMMUTABLE
- `s2_cell_from_token(token text) -> s2cellid`
  - token の解釈
  - IMMUTABLE

### 9.3 Bigint conversion（内部表現）

- `s2_cell_to_bigint(cell s2cellid) -> bigint`
  - `bigint` は `i64_norm`（order-preserving）を返す
  - IMMUTABLE
- `s2_cell_from_bigint(id bigint) -> s2cellid`
  - `id` を `i64_norm` として解釈し `s2cellid` に
  - IMMUTABLE

---

## 10. Hierarchical grid functions

### 10.1 Parent / child

- `s2_cell_to_parent(cell s2cellid, level integer) -> s2cellid`
  - 指定 level の親
  - IMMUTABLE
- `s2_cell_to_parent(cell s2cellid) -> s2cellid`
  - level-1 の親
  - IMMUTABLE
- `s2_cell_to_children(cell s2cellid, level integer) -> SETOF s2cellid`
  - 指定 level の子を列挙
  - IMMUTABLE
- `s2_cell_to_children(cell s2cellid) -> SETOF s2cellid`
  - level+1 の子
  - IMMUTABLE
- `s2_cell_to_center_child(cell s2cellid, level integer) -> s2cellid`
  - 指定 level の center child（存在する場合）
  - IMMUTABLE
- `s2_cell_to_center_child(cell s2cellid) -> s2cellid`
  - level+1 の center child
  - IMMUTABLE

### 10.2 Range（covering / filtering で重要）

- `s2_cell_range_min(cell s2cellid) -> s2cellid`
- `s2_cell_range_max(cell s2cellid) -> s2cellid`

返す CellID は **内部表現の順序（i64_norm order）** と整合すること。

IMMUTABLE

---

## 11. Traversal functions（MVP: limited）

S2 は六角格子ではないため、h3 の `grid_disk` を完全互換にするのは狙わない。  
ただし「近傍セル列挙」は実用性が高いので最小限を提供。

- `s2_cell_edge_neighbors(cell s2cellid) -> s2cellid[]`
  - 4 方向の edge neighbors
  - IMMUTABLE
- `s2_cell_all_neighbors(cell s2cellid) -> s2cellid[]`
  - 同 level の “all neighbors”（最大 8 を想定）
  - IMMUTABLE

> NOTE: “k-hop 近傍” は v0.2+（SHOULD）  
> 提供する場合は `s2_grid_disk(origin s2cellid, k integer = 1) -> SETOF s2cellid` で命名を寄せる。

---

## 12. Region functions（MVP）

PostGIS なしで「範囲検索の事前フィルタ」を作るため、cap / rect covering を提供する。

### 12.1 Cap covering（中心 + 半径）

- `s2_cover_cap(center point, radius_m double precision, level integer, max_cells integer = 8) -> SETOF s2cellid`
  - `center` を中心、`radius_m`（メートル）の円領域を覆うセル集合
  - 原則: `min_level = max_level = level`（固定レベル covering）
  - `max_cells` は RegionCoverer の制限
  - STABLE（max_cells や内部実装依存の揺れを避けるため IMMUTABLE を要求しない）
- `s2_cover_cap(center point, radius_m double precision) -> SETOF s2cellid`
  - default level を用いる（`pg_s2.default_cover_level`）
  - STABLE

### 12.2 Rect covering（bbox）

- `s2_cover_rect(rect box, level integer, max_cells integer = 8) -> SETOF s2cellid`
  - `box` は (lng_min, lat_min) と (lng_max, lat_max) の 2 点で与える
  - antimeridian 跨ぎは v0.2+（MAY）
  - STABLE
- `s2_cover_rect(rect box) -> SETOF s2cellid`
  - default cover level
  - STABLE

### 12.3 Ranges helper（事前フィルタ向け）

B-tree を効かせた WHERE 条件に落としやすくするため、range を返す関数を用意する。  
v0.1 では「セルごとの range」を返す（マージは v0.2+）。

- `s2_cover_cap_ranges(center point, radius_m double precision, level integer, max_cells integer = 8) -> SETOF int8range`
  - 各 covering cell について:
    - `[s2_cell_to_bigint(s2_cell_range_min(cell)), s2_cell_to_bigint(s2_cell_range_max(cell))]`
    - を `int8range`（両端含む）として返す
  - STABLE
- `s2_cover_rect_ranges(rect box, level integer, max_cells integer = 8) -> SETOF int8range`
  - STABLE

---

## 13. Misc functions（MVP）

### 13.1 Great-circle distance

- `s2_great_circle_distance(a point, b point, unit text = 'm') -> double precision`
  - a,b の球面距離を返す
  - `unit`: `'m' | 'km' | 'rad'`（MVP は 'm' と 'km' だけでも可）
  - IMMUTABLE

---

## 14. Casts / Operators / Index support

### 14.1 Casts

- `s2cellid :: text`（token）
- `text :: s2cellid`（token parsing）
- `s2cellid :: bigint`（= i64_norm）
- `bigint :: s2cellid`

### 14.2 Operators（MVP）

- 比較: `=`, `<>`, `<`, `<=`, `>`, `>=`
  - すべて **unsigned order に整合** すること（i64_norm により実現）
- 近傍や距離演算子は v0.2+（MAY）

### 14.3 B-tree opclass（MVP）

- `s2cellid` 用の B-tree オペレータクラスを提供
  - 比較は i64_norm の signed 比較で OK（= unsigned order）
- `s2cellid` カラムに対し `CREATE INDEX ... USING btree` が効くこと

---

## 15. 設定 (GUC)

- `pg_s2.default_level` (int, default: 14)
  - `s2_lat_lng_to_cell(point)` の level
- `pg_s2.default_cover_level` (int, default: 12)
  - `s2_cover_cap(... no level ...)` 等の level
- `pg_s2.earth_radius_m` (float8, default: 6371008.8)
  - distance / cap の meters↔radians 換算に使用
- `pg_s2.extend_antimeridian` (bool, default: false) — v0.2+（MAY）
  - boundary/rect で ±180° 跨ぎを拡張表現するか

---

## 16. 典型的な使い方（recipes）

### 16.1 点データをセル化して保存（B-tree）

```sql
CREATE EXTENSION pg_s2;

CREATE TABLE places (
  id bigserial PRIMARY KEY,
  name text NOT NULL,
  ll point NOT NULL,
  cell s2cellid GENERATED ALWAYS AS (s2_lat_lng_to_cell(ll, 14)) STORED,
  cell_i8 bigint GENERATED ALWAYS AS (s2_cell_to_bigint(cell)) STORED
);

CREATE INDEX ON places (cell);
CREATE INDEX ON places (cell_i8);
```

### 16.2 bbox の事前フィルタ（covering + ranges）

```sql
-- bbox (lng_min,lat_min)-(lng_max,lat_max)
WITH ranges AS (
  SELECT r
  FROM s2_cover_rect_ranges(box(point(139.60,35.60), point(139.80,35.75)), 12, 16) AS r
)
SELECT p.*
FROM places p
WHERE EXISTS (
  SELECT 1
  FROM ranges
  WHERE p.cell_i8 <@ ranges.r
);
```

> NOTE: 実運用では planner / index の効き方を見て
>
> - ranges を UNION ALL で展開して `p.cell_i8 BETWEEN lo AND hi`
> - あるいは `JOIN LATERAL` と `OR` 展開（クエリ生成）
>   等を選ぶ。v0.2+ で「range のマージ」機能を入れると改善する。

### 16.3 半径検索の事前フィルタ（cap covering）

```sql
WITH ranges AS (
  SELECT r
  FROM s2_cover_cap_ranges(point(139.75, 35.68), 2000.0, 12, 16) AS r
)
SELECT p.*
FROM places p
WHERE EXISTS (
  SELECT 1
  FROM ranges
  WHERE p.cell_i8 <@ ranges.r
);
```

---

## 17. 実装ノート（pgrx / Rust）

### 17.1 `s2cellid` の実装

- `s2cellid` は 8byte pass-by-value のカスタム型として実装する
- 内部値は `i64_norm`
- text I/O:
  - parse token -> u64 -> i64_norm
  - format: i64_norm -> u64 -> token

- 例外・エラーは Postgres エラーレベルで返す（pgrx の `error!` 等）

### 17.2 安定性（IMMUTABLE/STABLE）

- 数学的に決定的なものは IMMUTABLE
- covering は実装や丸めにより将来差分が出やすいので STABLE

---

## 18. テスト方針

- `cargo pgrx test` による回帰試験
- Golden vectors（既知の lat/lng と token の対応）を固定
- round-trip:
  - token -> cellid -> token
  - point -> cellid -> center point（誤差許容）
- range ordering:
  - parent range が child を包含する
  - `range_min <= child <= range_max` が unsigned order で成立

---

## 19. バージョニング / リリース

- v0.1.0: 本 SPEC の MVP（cellid + 基本変換 + 階層 + cap/rect covering + ranges）
- v0.2.x:
  - ranges のマージ（連続区間の統合）
  - antimeridian 対応（boundary/rect）
  - `s2_grid_disk(k)` 相当の追加
- v0.3+:
  - polygon covering（ただし PostGIS なし前提だと入力型を要検討）

---

## 20. 参考（非規範）

- h3-pg API Reference: [https://pgxn.org/dist/h3/docs/api.html](https://pgxn.org/dist/h3/docs/api.html)
- pgs2: [https://github.com/michelp/pgs2](https://github.com/michelp/pgs2)
- s2-postgis: [https://github.com/AfieldTrails/s2-postgis](https://github.com/AfieldTrails/s2-postgis)
- Rust S2 (candidate): [https://github.com/yjh0502/rust-s2](https://github.com/yjh0502/rust-s2)
- S2 Geometry: [https://s2geometry.io/](https://s2geometry.io/)
- BigQuery S2 (CellID signed int64 / token ordering などの知見): [https://cloud.google.com/bigquery/docs/reference/standard-sql/geography_functions#s2_cellid](https://cloud.google.com/bigquery/docs/reference/standard-sql/geography_functions#s2_cellid)
