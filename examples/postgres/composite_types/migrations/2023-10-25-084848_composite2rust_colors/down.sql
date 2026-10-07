-- This file should undo anything in `up.sql`
DROP TABLE colors;
DROP FUNCTION color2grey;
DROP FUNCTION color2gray;
DROP TYPE IF EXISTS nested_type;
DROP TYPE gray_type;
