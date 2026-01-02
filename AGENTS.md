# AGENTS.md

## 実装の進め方（このリポジトリ向け）

- Docker 経由でビルド/テストする前提。`make test` が標準。
- TDD を超細粒度で進める（1関数・1挙動単位）。
  - 例: テスト1本 → 実装 → `make test` → 次の関数。
- 既存の `.forks/pg_rrf` をベースに pgrx 拡張の構成を合わせる。
- `s2cellid` は固定長 `int8` 相当として扱い、内部表現は `i64_norm`（order-preserving）。
- 変換系は「token ↔ cellid」「bigint ↔ cellid」を独立に実装・検証する。

## テスト実行

- `make test`
  - Docker ビルド → `cargo pgrx test` をコンテナ内で実行

