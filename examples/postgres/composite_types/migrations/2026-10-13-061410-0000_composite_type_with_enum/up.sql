CREATE TYPE my_enum AS ENUM ('A', 'B');

CREATE TYPE composite_with_enum_field as (
    a               INTEGER,
    b               TEXT,
    my_enum_value   my_enum
);

COMMENT ON TYPE composite_with_enum_field IS 'composite_with_enum_field comment';

CREATE FUNCTION returns_composite_with_enum_field(
) RETURNS composite_with_enum_field AS
$$
BEGIN
    return (1::INTEGER, 'b'::TEXT, 'A'::my_enum);
END
$$ LANGUAGE plpgsql IMMUTABLE;
