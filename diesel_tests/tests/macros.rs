// FIXME: We need to support SQL functions on SQLite. The test itself will
// probably need to change to deal with how SQLite handles functions. I do not
// think we need to generically support creation of these functions, as it's
// different enough in SQLite to avoid.
#![cfg(feature = "postgres")]
use crate::schema::*;
use diesel::sql_types::{BigInt, VarChar};
use diesel::*;

#[declare_sql_function]
extern "SQL" {
    fn my_lower(x: VarChar) -> VarChar;
    fn setval(x: VarChar, y: BigInt);
    fn currval(x: VarChar) -> BigInt;
}

#[diesel_test_helper::test]
fn test_sql_function() {
    use crate::schema::users::dsl::*;

    let connection = &mut connection_with_sean_and_tess_in_users_table();
    diesel::sql_query(
        "CREATE FUNCTION my_lower(varchar) RETURNS varchar
        AS $$ SELECT LOWER($1) $$
        LANGUAGE SQL",
    )
    .execute(connection)
    .unwrap();
    let sean = User::new(1, "Sean");
    let tess = User::new(2, "Tess");

    assert_eq!(
        vec![sean],
        users
            .filter(my_lower(name).eq("sean"))
            .load(connection)
            .unwrap()
    );
    assert_eq!(
        vec![tess],
        users
            .filter(my_lower(name).eq("tess"))
            .load(connection)
            .unwrap()
    );
}

#[diesel_test_helper::test]
fn sql_function_without_return_type() {
    let connection = &mut connection();
    select(setval("users_id_seq", 54))
        .execute(connection)
        .unwrap();

    let seq_val = select(currval("users_id_seq")).get_result::<i64>(connection);
    assert_eq!(Ok(54), seq_val);
}

#[cfg(feature = "postgres")]
pub mod composite_return_type {
    use diesel::deserialize::FromSql;
    use diesel::pg::{Pg, PgValue};
    use diesel::query_source::QueryRelation;
    use diesel::{
        AppearsOnTable, FromSqlRow, QueryDsl, QuerySource, SelectableExpression, SqlType,
        declare_sql_function,
    };
    use insta::assert_snapshot;

    #[derive(SqlType)]
    #[diesel(postgres_type(name = "foo", schema = "public"))]
    pub struct PgFoo;

    pub mod utils {
        use diesel::Expression;
        use diesel::sql_types::*;

        pub struct a;
        pub struct b;
        pub struct c;

        impl Expression for a {
            type SqlType = BigInt;
        }

        impl Expression for b {
            type SqlType = Integer;
        }

        impl Expression for c {
            type SqlType = Text;
        }

        pub type AllColumns = (a, b, c);
        pub const all_columns: AllColumns = (a, b, c);

        pub type SqlType = <AllColumns as diesel::Expression>::SqlType;
    }

    #[derive(Debug, Clone, FromSqlRow)]
    pub struct Foo {
        a: i64,
        b: i32,
        c: String,
    }

    impl FromSql<PgFoo, Pg> for Foo {
        fn from_sql(bytes: PgValue) -> diesel::deserialize::Result<Self> {
            let (a, b, c) =
                FromSql::<diesel::sql_types::Record<utils::SqlType>, Pg>::from_sql(bytes)?;

            Ok(Self { a, b, c })
        }
    }

    impl QuerySource for foo {
        type FromClause = Self;
        type DefaultSelection = utils::AllColumns;

        fn from_clause(&self) -> Self::FromClause {
            *self
        }

        fn default_selection(&self) -> Self::DefaultSelection {
            all_columns
        }
    }

    impl QueryRelation for foo {
        type AllColumns = utils::AllColumns;

        fn all_columns() -> Self::AllColumns {
            all_columns
        }
    }

    impl AppearsOnTable<foo> for utils::a {}
    impl AppearsOnTable<foo> for utils::b {}
    impl AppearsOnTable<foo> for utils::c {}

    impl SelectableExpression<foo> for utils::a {}
    impl SelectableExpression<foo> for utils::b {}
    impl SelectableExpression<foo> for utils::c {}

    #[declare_sql_function]
    extern "SQL" {
        fn foo() -> PgFoo;
    }

    #[test]
    fn select_query() {
        let query = diesel::select(foo());

        assert_snapshot!(diesel::debug_query::<Pg, _>(&query));
    }

    #[test]
    fn select_from() {
        let query = foo().select(all_columns);

        assert_snapshot!(diesel::debug_query::<Pg, _>(&query));
    }
}
