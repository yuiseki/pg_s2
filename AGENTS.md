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

## バージョンアップ手順

1. `Cargo.toml` の `version` を更新
2. `META.json` の `version` / `provides.pg_s2.version` を更新
3. `README.md` の Status Version を更新
4. `make test` を実行
5. コミット → tag（例: `v0.0.3`）→ push
6. GitHub Release を作成（tag と同名）

### ルール

- 一度切ったバージョンは絶対に上書きしない（必ず version bump する）
