use composite_types::establish_connection;
use composite_types::schema::rust_types::MyEnum;
use composite_types::schema::sql_types::CompositeWithEnumField as PgCompositeWithEnumField;
use diesel::deserialize::{FromSql, FromSqlRow};
use diesel::dsl::select;
use diesel::pg::{Pg, PgValue};
use diesel::sql_types::{CompositeType, Record};
use diesel::{Expression, RunQueryDsl, declare_sql_function};

#[declare_sql_function]
extern "SQL" {
    fn returns_composite_with_enum_field() -> PgCompositeWithEnumField;
}

#[derive(Debug, FromSqlRow, PartialEq)]
struct CompositeWithEnumField {
    pub a: i32,
    pub b: Option<String>,
    pub my_enum_value: Option<MyEnum>,
}

impl FromSql<PgCompositeWithEnumField, Pg> for CompositeWithEnumField {
    fn from_sql(bytes: PgValue) -> diesel::deserialize::Result<Self> {
        let (a, b, my_enum_value) = FromSql::<
            Record<<<PgCompositeWithEnumField as CompositeType>::AllFields as Expression>::SqlType>,
            Pg,
        >::from_sql(bytes)?;
        Ok(CompositeWithEnumField {
            a,
            b,
            my_enum_value,
        })
    }
}

fn main() {
    let connection = &mut establish_connection();
    let result: CompositeWithEnumField = select(returns_composite_with_enum_field())
        .get_result(connection)
        .unwrap();
    println!("{result:?}");
}

#[cfg(test)]
mod tests {
    use composite_types::establish_connection;
    use composite_types::schema::rust_types::MyEnum;
    use diesel::pg::Pg;
    use diesel::{RunQueryDsl, debug_query, select};

    #[test]
    fn fn_call_query() {
        let query = select(super::returns_composite_with_enum_field());

        assert_eq!(
            "SELECT returns_composite_with_enum_field() -- binds: []",
            debug_query::<Pg, _>(&query).to_string()
        );
    }

    #[test]
    fn fn_call() {
        let connection = &mut establish_connection();
        let result: super::CompositeWithEnumField =
            select(super::returns_composite_with_enum_field())
                .get_result(connection)
                .unwrap();

        assert_eq!(
            super::CompositeWithEnumField {
                a: 1,
                b: Some("b".to_string()),
                my_enum_value: Some(MyEnum::A)
            },
            result
        );
    }
}
