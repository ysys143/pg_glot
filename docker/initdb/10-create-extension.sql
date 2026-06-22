-- 첫 클러스터 초기화 시 전체 스택을 자동 생성한다(편의).
-- pg_textsearch는 이미지에서 shared_preload_libraries로 preload돼 있으므로
-- 이 시점에 사용 가능하다. CASCADE로 pg_tsvector_ko + pg_textsearch + vector가 함께 생성된다.
CREATE EXTENSION IF NOT EXISTS pg_textsearch_ko CASCADE;
