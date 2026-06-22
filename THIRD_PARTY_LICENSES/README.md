# Third-Party Licenses

`pg_textsearch_ko` itself is under the **PostgreSQL License** (see `../LICENSE`).

When distributed as a **bundle** (e.g. the Docker image), the binaries below are
included and their license/notice texts are reproduced here to satisfy attribution
obligations. All are permissive (no copyleft). See `../docs/DESIGN.md` §6 for the
full audit.

| Component | Role | License | Text |
|---|---|---|---|
| lindera | morphological engine (compiled into `pg_tsvector_ko`) | MIT | `lindera.LICENSE.txt` |
| lindera-ko-dic (mecab-ko-dic-2.1.1-20180720) | embedded dictionary data | Apache-2.0 | `Apache-2.0.LICENSE.txt` + `ko-dic.NOTICE.txt` |
| pg_textsearch (Timescale/Tiger Data) | BM25 index | PostgreSQL License | `pg_textsearch.LICENSE.txt` |
| pgvector | dense vector search | PostgreSQL License | `pgvector.LICENSE.txt` |
| PostgreSQL | database server (Docker base image) | PostgreSQL License | provided by the `postgres` base image |

Notes:
- **Kiwi (LGPL-2.1+)** is NOT bundled. It is an opt-in build feature, dynamically
  linked only when explicitly enabled; default builds and the default image contain
  no LGPL code.
- `pg_textsearch` is a trademark of its authors (Timescale/Tiger Data). This bundle
  redistributes it under the PostgreSQL License; it is not an official Timescale
  product.
- Benchmark corpora (MIRACL/MDN/EZIS) are NOT included in source or image.
