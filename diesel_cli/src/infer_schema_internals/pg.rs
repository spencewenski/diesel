use super::data_structures::*;
use super::information_schema::DefaultSchema;
use super::{CompositeTypeData, TableName, rust_name_for_sql_name};
use crate::config::CompositeTypeConfig;
use crate::print_schema::ColumnSorting;
use diesel::connection::DefaultLoadingMode;
use diesel::dsl::AsExprOf;
use diesel::pg::Pg;
use diesel::prelude::table;
use diesel::prelude::*;
use diesel::sql_types;
use heck::ToUpperCamelCase;
use std::borrow::Cow;

#[diesel::declare_sql_function]
extern "SQL" {
    #[aggregate]
    fn array_agg(input: sql_types::Text) -> sql_types::Array<sql_types::Text>;

    fn col_description(
        table: sql_types::Oid,
        column_number: sql_types::BigInt,
    ) -> sql_types::Nullable<sql_types::Text>;

    fn obj_description(
        oid: sql_types::Oid,
        catalog: sql_types::Text,
    ) -> sql_types::Nullable<sql_types::Text>;

    fn quote_ident(ident: sql_types::Text) -> sql_types::Text;
}

mod information_schema {
    use diesel::prelude::table;

    table! {
        information_schema.columns (table_schema, table_name, column_name) {
            table_schema -> VarChar,
            table_name -> VarChar,
            column_name -> VarChar,
            #[sql_name = "is_nullable"]
            __is_nullable -> VarChar,
            character_maximum_length -> Nullable<Integer>,
            ordinal_position -> BigInt,
            udt_name -> VarChar,
            udt_schema -> VarChar,
            domain_name -> Nullable<VarChar>,
            domain_schema -> Nullable<VarChar>,
        }
    }

    table! {
        information_schema.attributes (udt_schema, udt_name, attribute_name) {
            udt_schema -> VarChar,
            udt_name -> VarChar,
            attribute_name -> VarChar,
            ordinal_position -> Integer,
            #[sql_name = "is_nullable"]
            __is_nullable -> VarChar,
            attribute_udt_schema -> VarChar,
            attribute_udt_name -> VarChar,
            character_maximum_length -> Nullable<Integer>,
        }
    }
}

table! {
    pg_type(oid) {
        oid -> Oid,
        typname -> Text,
        typnamespace -> Oid,
    }
}

table! {
    pg_enum(oid) {
        oid -> Oid,
        enumtypid -> Oid,
        enumsortorder -> Float,
        enumlabel -> Text,
    }
}

table! {
    pg_namespace(oid) {
        oid -> Oid,
        nspname -> Text,
    }
}

allow_tables_to_appear_in_same_query!(pg_type, pg_enum);
allow_tables_to_appear_in_same_query!(information_schema::attributes, pg_type, pg_namespace);

#[tracing::instrument]
pub fn determine_column_type(
    attr: &ColumnInformation,
    default_schema: &str,
) -> Result<ColumnType, crate::errors::Error> {
    let is_array = attr.type_name.starts_with('_');
    let tpe = if is_array {
        &attr.type_name[1..]
    } else {
        &attr.type_name
    };

    let diesel_alias_without_postgres_coercion = match &*tpe.to_lowercase() {
        "varchar" | "citext" => Some(tpe),
        _ => None,
    };

    // Postgres doesn't coerce varchar[] to text[] so print out a message to inform
    // the user.
    if let (true, Some(tpe)) = (is_array, diesel_alias_without_postgres_coercion) {
        tracing::info!("Cannot coerce varchar[] into text[]");
        eprintln!(
            "The column `{}` is of type `{}[]`. This will cause problems when using Diesel. You should consider changing the column type to `text[]`.",
            attr.column_name, tpe
        );
    }

    Ok(ColumnType {
        schema: attr.type_schema.as_ref().and_then(|s| {
            if s == default_schema {
                None
            } else {
                Some(s.clone())
            }
        }),
        sql_name: tpe.to_string(),
        rust_name: tpe.to_upper_camel_case(),
        is_array,
        is_nullable: attr.nullable,
        is_unsigned: false,
        record: None,
        max_length: attr.max_length,
        unmodified_type: attr.type_name.clone(),
    })
}

diesel::postfix_operator!(Regclass, "::regclass", sql_types::Oid, backend: Pg);

fn regclass<'a, QS>(
    table: &'a TableName,
) -> Regclass<Box<dyn BoxableExpression<QS, Pg, SqlType = sql_types::Text> + 'a>> {
    let table_name = match table.schema {
        Some(ref schema_name) => Box::new(
            quote_ident(schema_name)
                .concat(".")
                .concat(quote_ident(&table.sql_name)),
        )
            as Box<dyn BoxableExpression<_, _, SqlType = sql_types::Text>>,
        None => Box::new(quote_ident(&table.sql_name)),
    };

    Regclass::new(table_name)
}

diesel::postfix_operator!(RegNamespace, "::regnamespace", sql_types::Oid, backend: Pg);

fn regnamespace(schema: &str) -> RegNamespace<quote_ident<AsExprOf<&str, sql_types::Text>>> {
    RegNamespace::new(quote_ident(schema))
}

pub fn get_table_data(
    conn: &mut PgConnection,
    table: &TableName,
    column_sorting: &ColumnSorting,
    domains_as_custom_types: &[&regex::Regex],
) -> QueryResult<Vec<ColumnInformation>> {
    use self::information_schema::columns::dsl::*;

    #[derive(Queryable)]
    struct Row {
        column_name: String,
        type_name: String,
        type_schema: Option<String>,
        domain_name: Option<String>,
        domain_schema: Option<String>,
        nullable: String,
        max_length: Option<i32>,
        comment: Option<String>,
    }

    let schema_name = match table.schema {
        Some(ref name) => Cow::Borrowed(name),
        None => Cow::Owned(Pg::default_schema(conn)?),
    };

    let query = columns
        .select((
            column_name,
            udt_name,
            udt_schema.nullable(),
            domain_name,
            domain_schema.nullable(),
            __is_nullable,
            character_maximum_length,
            col_description(regclass(table), ordinal_position),
        ))
        .filter(table_name.eq(&table.sql_name))
        .filter(table_schema.eq(schema_name));

    let rows: Vec<Row> = match column_sorting {
        ColumnSorting::OrdinalPosition => query.order(ordinal_position).load(conn),
        ColumnSorting::Name => query.order(column_name).load(conn),
    }?;

    rows.into_iter()
        .map(|row| {
            let (type_name, type_schema) = row
                .domain_name
                .filter(|name| {
                    domains_as_custom_types
                        .iter()
                        .any(|regex| regex.is_match(name))
                })
                .map(|name| (name, row.domain_schema))
                .unwrap_or((row.type_name, row.type_schema));

            let max_length = row.max_length.map(max_len_to_u64).transpose()?;

            Ok(ColumnInformation::new(
                row.column_name,
                type_name,
                type_schema,
                row.nullable == "YES",
                max_length,
                row.comment,
                false,
            ))
        })
        .collect()
}

fn max_len_to_u64(len: i32) -> Result<u64, diesel::result::Error> {
    len.try_into().map_err(|e| {
        diesel::result::Error::DeserializationError(
            format!("Max column length can't be converted to u64: {e} (got: {len})").into(),
        )
    })
}

pub fn get_table_comment(
    conn: &mut PgConnection,
    table: &TableName,
) -> QueryResult<Option<String>> {
    diesel::select(obj_description(regclass(table), "pg_class")).get_result(conn)
}

#[allow(clippy::similar_names)]
pub fn load_foreign_key_constraints(
    connection: &mut PgConnection,
    schema_name: Option<&str>,
) -> QueryResult<Vec<ForeignKeyConstraint>> {
    #[derive(QueryableByName)]
    struct ForeignKeyList {
        #[diesel(sql_type = sql_types::Text)]
        self_schema: String,
        #[diesel(sql_type = sql_types::Text)]
        self_table: String,
        #[diesel(sql_type = sql_types::Array<sql_types::Text>)]
        self_columns: Vec<String>,
        #[diesel(sql_type = sql_types::Text)]
        foreign_schema: String,
        #[diesel(sql_type = sql_types::Text)]
        foreign_table: String,
        #[diesel(sql_type = sql_types::Array<sql_types::Text>)]
        foreign_columns: Vec<String>,
    }

    let default_schema = Pg::default_schema(connection)?;
    let schema_name = schema_name.unwrap_or(&default_schema);

    diesel::sql_query(include_str!("load_foreign_keys.sql"))
        .bind::<sql_types::Text, _>(schema_name)
        .load_iter::<ForeignKeyList, DefaultLoadingMode>(connection)?
        .map(|f| {
            let f = f?;
            let mut child_table = TableName::new(f.self_table, f.self_schema);
            child_table.strip_schema_if_matches(&default_schema);
            let mut parent_table = TableName::new(f.foreign_table, f.foreign_schema);
            parent_table.strip_schema_if_matches(&default_schema);

            let foreign_key_columns_rust = f
                .self_columns
                .iter()
                .map(|s| super::inference::rust_name_for_sql_name(s, Some(&child_table)))
                .collect();
            Ok(ForeignKeyConstraint {
                child_table,
                parent_table,
                foreign_key_columns: f.self_columns,
                foreign_key_columns_rust,
                primary_key_columns: f.foreign_columns,
            })
        })
        .collect()
}

pub fn load_enum_variants(
    conn: &mut PgConnection,
    enum_name: &str,
    schema_name: Option<&str>,
) -> QueryResult<Option<Vec<EnumVariant>>> {
    let default_schema = Pg::default_schema(conn)?;

    let r = pg_enum::table
        .select((
            pg_enum::enumsortorder.cast::<diesel::sql_types::Integer>(),
            pg_enum::enumlabel,
        ))
        .filter(
            pg_enum::enumtypid.nullable().eq(pg_type::table
                .select(pg_type::oid)
                .filter(pg_type::typname.eq(enum_name))
                .filter(
                    pg_type::typnamespace.eq(regnamespace(schema_name.unwrap_or(&default_schema))),
                )
                .single_value()),
        )
        .order_by(pg_enum::enumsortorder)
        .load::<EnumVariant>(conn)?;
    if r.is_empty() { Ok(None) } else { Ok(Some(r)) }
}

pub fn get_type_comment(
    conn: &mut PgConnection,
    type_name: &str,
    schema_name: Option<&str>,
) -> QueryResult<Option<String>> {
    let default_schema = Pg::default_schema(conn)?;

    pg_type::table
        .select(obj_description(pg_type::oid, "pg_type"))
        .filter(pg_type::typname.eq(type_name))
        .filter(pg_type::typnamespace.eq(regnamespace(schema_name.unwrap_or(&default_schema))))
        .get_result::<Option<String>>(conn)
}

pub fn load_composite_types(
    conn: &mut PgConnection,
    config: &CompositeTypeConfig,
) -> Result<Vec<QueryRelationData>, crate::errors::Error> {
    use crate::infer_schema_internals::pg::information_schema::attributes;
    use crate::infer_schema_internals::pg::pg_namespace;
    use crate::infer_schema_internals::pg::pg_type;

    #[derive(Queryable, Selectable, PartialEq, Debug, Clone)]
    #[diesel(table_name = attributes)]
    #[diesel(check_for_backend(diesel::pg::Pg))]
    pub struct CompositeTypeAttribute {
        udt_schema: String,
        udt_name: String,
        attribute_name: String,
        ordinal_position: i32,
        __is_nullable: String,
        attribute_udt_schema: String,
        attribute_udt_name: String,
        character_maximum_length: Option<i32>,
    }

    let default_schema = Pg::default_schema(conn)?;

    // Load all composite type attributes from the `information_schema.attributes` table. Join with
    // the `pg_type` and `pg_namespace` tables in order to determine the correct oid to use with the
    // `obj_description` function in order to fetch comments set on the composite types.
    let attrs = attributes::table
        .inner_join(pg_type::table.on(attributes::udt_name.eq(pg_type::typname)))
        .inner_join(pg_namespace::table.on(pg_type::typnamespace.eq(pg_namespace::oid)))
        .filter(attributes::udt_schema.eq(pg_namespace::nspname))
        .select((
            CompositeTypeAttribute::as_select(),
            obj_description(pg_type::oid, "pg_type"),
        ))
        .order_by((
            attributes::udt_schema,
            attributes::udt_name,
            attributes::ordinal_position,
        ))
        .load::<(CompositeTypeAttribute, Option<String>)>(conn)?;

    // Create a `ColumnDefinition` for each attribute
    let attrs = attrs
        .into_iter()
        .map(|(attr, composite_type_comment)| {
            let max_length = attr
                .character_maximum_length
                .map(max_len_to_u64)
                .transpose()?;

            let attr_information = ColumnInformation::new(
                attr.attribute_name,
                attr.attribute_udt_name,
                Some(attr.attribute_udt_schema),
                attr.__is_nullable == "YES",
                max_length,
                None,
                false,
            );

            let attr_definition = ColumnDefinition {
                ty: determine_column_type(&attr_information, &default_schema)?,
                rust_name: rust_name_for_sql_name(&attr_information.column_name, None),
                sql_name: attr_information.column_name,
                comment: attr_information.comment,
                auto_increment: attr_information.auto_increment,
            };

            Ok((
                // Pass along properties that belong to the composite type. These are used to
                // initialize the `CompositeTypeData` below.
                (attr.udt_schema, attr.udt_name, composite_type_comment),
                attr_definition,
            ))
        })
        .collect::<Result<Vec<_>, crate::errors::Error>>()?;

    let composite_types = attrs.into_iter().fold(
        Vec::<CompositeTypeData>::new(),
        |mut accum, ((schema, name, composite_type_comment), field)| {
            // If the last `CompositeTypeData` in the result has the same schema+name as the current
            // schema+name, then pop it from the result so we can add the current attribute to its
            // `field_data`. Otherwise, create a new `CompositeTypeData`.
            // Assumes the attributes are ordered by schema, name, and ordinal.
            let mut ty = match accum.last() {
                Some(ty)
                    if let Some(ty_schema) = ty.name.schema.as_deref()
                        && ty_schema == schema
                        && ty.name.sql_name == name =>
                {
                    accum.pop()
                }
                _ => None,
            }
            .unwrap_or_else(|| CompositeTypeData {
                name: TableName::new(name, schema),
                comment: composite_type_comment,
                field_data: vec![],
            });

            ty.field_data.push(field);
            accum.push(ty);
            accum
        },
    );

    let composite_types = composite_types
        .into_iter()
        .map(|mut ty| {
            ty.name.strip_schema_if_matches(&default_schema);
            ty
        })
        .map(|ty| (ty.name.full_sql_name(), ty))
        .filter(|(full_sql_name, _)| {
            config.include.is_empty() || config.include.iter().any(|r| r.is_match(full_sql_name))
        })
        .filter(|(full_sql_name, _)| !config.exclude.iter().any(|r| r.is_match(full_sql_name)))
        .map(|(full_sql_name, mut ty)| {
            ty.field_data.iter_mut().for_each(|field| {
                let full_field_name = format!("{}.{}", full_sql_name, field.sql_name);
                let is_nullable = !config
                    .non_null_fields
                    .iter()
                    .any(|r| r.is_match(&full_field_name));
                field.ty.is_nullable &= is_nullable;
            });
            (full_sql_name, ty)
        })
        .map(|(_, ty)| QueryRelationData::CompositeType(ty))
        .collect();

    Ok(composite_types)
}

#[cfg(test)]
mod test {
    extern crate dotenvy;

    use self::dotenvy::dotenv;
    use super::*;
    use insta::assert_debug_snapshot;
    use regex::Regex;
    use std::env;

    fn connection() -> PgConnection {
        dotenv().ok();

        let connection_url = env::var("PG_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .expect("DATABASE_URL must be set in order to run tests");
        let mut connection = PgConnection::establish(&connection_url).unwrap();
        connection.begin_test_transaction().unwrap();
        connection
    }

    #[test]
    fn get_table_data_loads_column_information() {
        let mut connection = connection();

        diesel::sql_query("CREATE SCHEMA test_schema")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query(
                "CREATE TABLE test_schema.table_1 (id SERIAL PRIMARY KEY, text_col VARCHAR(128), not_null TEXT NOT NULL)",
            ).execute(&mut connection)
            .unwrap();
        diesel::sql_query("COMMENT ON COLUMN test_schema.table_1.id IS 'column comment'")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TABLE test_schema.table_2 (array_col VARCHAR[] NOT NULL)")
            .execute(&mut connection)
            .unwrap();

        let table_1 = TableName::new("table_1", "test_schema");
        let table_2 = TableName::new("table_2", "test_schema");
        let pg_catalog = Some(String::from("pg_catalog"));
        let id = ColumnInformation::new(
            "id",
            "int4",
            pg_catalog.clone(),
            false,
            None,
            Some("column comment".to_string()),
            false,
        );
        let text_col = ColumnInformation::new(
            "text_col",
            "varchar",
            pg_catalog.clone(),
            true,
            Some(128),
            None,
            false,
        );
        let not_null = ColumnInformation::new(
            "not_null",
            "text",
            pg_catalog.clone(),
            false,
            None,
            None,
            false,
        );
        let array_col = ColumnInformation::new(
            "array_col",
            "_varchar",
            pg_catalog,
            false,
            None,
            None,
            false,
        );
        assert_eq!(
            Ok(vec![id, text_col, not_null]),
            get_table_data(
                &mut connection,
                &table_1,
                &ColumnSorting::OrdinalPosition,
                &[]
            )
        );
        assert_eq!(
            Ok(vec![array_col]),
            get_table_data(
                &mut connection,
                &table_2,
                &ColumnSorting::OrdinalPosition,
                &[]
            )
        );
    }

    #[test]
    fn gets_table_comment() {
        let mut connection = connection();

        diesel::sql_query("CREATE SCHEMA test_schema")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query(
                "CREATE TABLE test_schema.table_1 (id SERIAL PRIMARY KEY, text_col VARCHAR, not_null TEXT NOT NULL)",
            ).execute(&mut connection)
            .unwrap();
        diesel::sql_query("COMMENT ON TABLE test_schema.table_1 IS 'table comment'")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TABLE test_schema.table_2 (array_col VARCHAR[] NOT NULL)")
            .execute(&mut connection)
            .unwrap();

        let table_1 = TableName::new("table_1", "test_schema");
        let table_2 = TableName::new("table_2", "test_schema");
        assert_eq!(
            Ok(Some("table comment".to_string())),
            get_table_comment(&mut connection, &table_1)
        );
        assert_eq!(Ok(None), get_table_comment(&mut connection, &table_2));
    }

    #[test]
    fn gets_type_comment() {
        let mut connection = connection();

        diesel::sql_query("CREATE SCHEMA test_schema")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TYPE commented_enum AS ENUM ('a', 'b')")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("COMMENT ON TYPE commented_enum IS 'enum comment'")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TYPE test_schema.commented_enum AS ENUM ('c')")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("COMMENT ON TYPE test_schema.commented_enum IS 'schema enum comment'")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TYPE uncommented_enum AS ENUM ('d')")
            .execute(&mut connection)
            .unwrap();

        assert_eq!(
            Ok(Some("enum comment".to_string())),
            get_type_comment(&mut connection, "commented_enum", None)
        );
        assert_eq!(
            Ok(Some("schema enum comment".to_string())),
            get_type_comment(&mut connection, "commented_enum", Some("test_schema"))
        );
        assert_eq!(
            Ok(None),
            get_type_comment(&mut connection, "uncommented_enum", None)
        );
        assert_eq!(
            Err(diesel::result::Error::NotFound),
            get_type_comment(&mut connection, "non_existing", None)
        );
    }

    #[test]
    fn get_foreign_keys_loads_foreign_keys() {
        let mut connection = connection();

        diesel::sql_query("CREATE SCHEMA test_schema")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TABLE test_schema.table_1 (id SERIAL PRIMARY KEY)")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query(
                "CREATE TABLE test_schema.table_2 (id SERIAL PRIMARY KEY, fk_one INTEGER NOT NULL REFERENCES test_schema.table_1)",
            ).execute(&mut connection)
            .unwrap();
        diesel::sql_query(
                "CREATE TABLE test_schema.table_3 (id SERIAL PRIMARY KEY, fk_two INTEGER NOT NULL REFERENCES test_schema.table_2)",
            ).execute(&mut connection)
            .unwrap();

        let table_1 = TableName::new("table_1", "test_schema");
        let table_2 = TableName::new("table_2", "test_schema");
        let table_3 = TableName::new("table_3", "test_schema");
        let fk_one = ForeignKeyConstraint {
            child_table: table_2.clone(),
            parent_table: table_1,
            foreign_key_columns: vec!["fk_one".into()],
            foreign_key_columns_rust: vec!["fk_one".into()],
            primary_key_columns: vec!["id".into()],
        };
        let fk_two = ForeignKeyConstraint {
            child_table: table_3,
            parent_table: table_2,
            foreign_key_columns: vec!["fk_two".into()],
            foreign_key_columns_rust: vec!["fk_two".into()],
            primary_key_columns: vec!["id".into()],
        };
        assert_eq!(
            Ok(vec![fk_one, fk_two]),
            load_foreign_key_constraints(&mut connection, Some("test_schema"))
        );
    }

    #[test]
    fn get_foreign_keys_loads_foreign_keys_with_same_name() {
        let mut connection = connection();

        diesel::sql_query("CREATE SCHEMA test_schema")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TABLE test_schema.table_1 (id SERIAL PRIMARY KEY)")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query(
            "CREATE TABLE test_schema.table_2 (\
                    id SERIAL PRIMARY KEY,\
                    fk_id INTEGER NOT NULL,\
                    CONSTRAINT fk FOREIGN KEY (fk_id) REFERENCES test_schema.table_1 (id))",
        )
        .execute(&mut connection)
        .unwrap();
        diesel::sql_query(
            "CREATE TABLE test_schema.table_3 (\
                    id SERIAL PRIMARY KEY,\
                    fk_id INTEGER NOT NULL,\
                    CONSTRAINT fk FOREIGN KEY (fk_id) REFERENCES test_schema.table_1 (id))",
        )
        .execute(&mut connection)
        .unwrap();

        let table_1 = TableName::new("table_1", "test_schema");
        let table_2 = TableName::new("table_2", "test_schema");
        let table_3 = TableName::new("table_3", "test_schema");
        let fk_one = ForeignKeyConstraint {
            child_table: table_2,
            parent_table: table_1.clone(),
            foreign_key_columns: vec!["fk_id".into()],
            foreign_key_columns_rust: vec!["fk_id".into()],
            primary_key_columns: vec!["id".into()],
        };
        let fk_two = ForeignKeyConstraint {
            child_table: table_3,
            parent_table: table_1,
            foreign_key_columns: vec!["fk_id".into()],
            foreign_key_columns_rust: vec!["fk_id".into()],
            primary_key_columns: vec!["id".into()],
        };
        assert_eq!(
            Ok(vec![fk_one, fk_two]),
            load_foreign_key_constraints(&mut connection, Some("test_schema"))
        );
    }

    #[test]
    fn get_table_data_considers_domain_types() {
        let mut connection = connection();

        diesel::sql_query("CREATE SCHEMA test_schema")
            .execute(&mut connection)
            .unwrap();

        diesel::sql_query("CREATE DOMAIN posinteger AS integer CHECK (VALUE > 0)")
            .execute(&mut connection)
            .unwrap();

        diesel::sql_query("CREATE TABLE test_schema.table_1 (id posinteger PRIMARY KEY)")
            .execute(&mut connection)
            .unwrap();

        let table_1 = TableName::new("table_1", "test_schema");

        let id_int = ColumnInformation::new(
            "id",
            "int4",
            Some(String::from("pg_catalog")),
            false,
            None,
            None,
            false,
        );
        let id_domain = ColumnInformation::new(
            "id",
            "posinteger",
            Some(String::from("public")),
            false,
            None,
            None,
            false,
        );

        assert_eq!(
            Ok(vec![id_int.clone()]),
            get_table_data(
                &mut connection,
                &table_1,
                &ColumnSorting::OrdinalPosition,
                &[]
            )
        );

        assert_eq!(
            Ok(vec![id_int.clone()]),
            get_table_data(
                &mut connection,
                &table_1,
                &ColumnSorting::OrdinalPosition,
                &[&"non-matching-regex".try_into().unwrap()]
            )
        );

        assert_eq!(
            Ok(vec![id_domain]),
            get_table_data(
                &mut connection,
                &table_1,
                &ColumnSorting::OrdinalPosition,
                &[&"int".try_into().unwrap()]
            )
        );
    }

    #[test]
    fn load_enum_variants() {
        let mut connection = connection();

        diesel::sql_query("CREATE TYPE test AS ENUM ('A', 'B')")
            .execute(&mut connection)
            .unwrap();

        let variants = super::load_enum_variants(&mut connection, "test", None).unwrap();
        assert!(variants.is_some());
        assert_eq!(
            variants.unwrap(),
            [
                EnumVariant {
                    order: 1,
                    sql_name: "A".into()
                },
                EnumVariant {
                    order: 2,
                    sql_name: "B".into()
                }
            ]
        );

        let variants = super::load_enum_variants(&mut connection, "non_existing", None).unwrap();
        assert!(variants.is_none());
    }

    #[test]
    fn load_composite_types() {
        let mut connection = connection();

        let config = CompositeTypeConfig {
            // Only print the types we create below
            include: vec![serde_regex::Serde(
                Regex::new("\\b(test_type)\\b$").unwrap(),
            )],
            // Ignore types in the `ignore_me` schema
            exclude: vec![serde_regex::Serde(Regex::new("^ignore_me\\..*").unwrap())],
            // Override fields of `test_schema.test_type` to be not nullable
            non_null_fields: vec![serde_regex::Serde(
                Regex::new("test_schema\\.test_type\\..*").unwrap(),
            )],
            ..Default::default()
        };

        diesel::sql_query("CREATE TYPE test_type AS (a INTEGER, b TEXT)")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE schema test_schema;")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TYPE test_schema.test_type AS (c REAL, d BOOLEAN)")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("COMMENT ON TYPE test_type IS 'test_type comment'")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query(
            "COMMENT ON TYPE test_schema.test_type IS 'test_schema.test_type comment'",
        )
        .execute(&mut connection)
        .unwrap();
        diesel::sql_query("CREATE schema ignore_me;")
            .execute(&mut connection)
            .unwrap();
        diesel::sql_query("CREATE TYPE ignore_me.test_type AS (a INTEGER, b TEXT)")
            .execute(&mut connection)
            .unwrap();

        let types = super::load_composite_types(&mut connection, &config)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();
        assert!(!types.is_empty());
        assert_debug_snapshot!(types);
    }
}
