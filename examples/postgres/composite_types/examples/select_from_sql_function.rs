use crate::schema::{grey_type, intensity};
use composite_types::establish_connection;
use diesel::sql_types::Integer;
use diesel::{QueryDsl, Queryable, RunQueryDsl, select};
use diesel::{SelectableHelper, declare_sql_function};

#[declare_sql_function]
extern "SQL" {
    // Proposal: New annotation to opt-in a function to generating the below trait implementations
    // #[query_source]
    fn color2grey(r: Integer, g: Integer, b: Integer) -> grey_type;
}

/// Module containing the required implementations for the `color2grey` function
///
/// Proposal: Generate these trait implementations from the `declare_sql_function` macro for
///           functions annotated with `#[query_source]`
mod color2grey_impls {
    use super::schema::grey_type;
    use diesel::query_builder::AsQuery;
    use diesel::query_source::{Function, QueryRelation};
    use diesel::sql_types::CompositeType;
    use diesel::{Expression, QuerySource};

    impl<R: Copy, G: Copy, B: Copy> QuerySource for super::color2grey_utils::color2grey<R, G, B> {
        type FromClause = Self;
        type DefaultSelection = <grey_type as CompositeType>::AllFields;

        fn from_clause(&self) -> Self::FromClause {
            *self
        }

        fn default_selection(&self) -> Self::DefaultSelection {
            <grey_type as CompositeType>::all_fields()
        }
    }

    impl<R: Copy, G: Copy, B: Copy> AsQuery for super::color2grey_utils::color2grey<R, G, B> {
        type SqlType = <<grey_type as CompositeType>::AllFields as Expression>::SqlType;
        type Query = diesel::internal::table_macro::SelectStatement<
            diesel::internal::table_macro::FromClause<Self>,
        >;

        fn as_query(self) -> Self::Query {
            diesel::internal::table_macro::SelectStatement::simple(self)
        }
    }

    impl<R: Copy, G: Copy, B: Copy> QueryRelation for super::color2grey_utils::color2grey<R, G, B> {
        type AllColumns = <grey_type as CompositeType>::AllFields;

        fn all_columns() -> Self::AllColumns {
            <grey_type as CompositeType>::all_fields()
        }
    }

    impl<R: Copy, G: Copy, B: Copy> Function for super::color2grey_utils::color2grey<R, G, B> {
        type Return = grey_type;
    }
}

#[derive(Debug, Clone, Queryable)]
// Proposal: Update the `Selectable` derive macro to include the below implementations
// #[derive(Selectable)]
pub struct GreyType {
    pub intensity: f32,
    pub suggestion: String,
}

/// Module containing the required implementations for the `GreyType` rust struct
///
/// Proposal: Update the `Selectable` derive to allow specifying a composite type instead of a table
/// Proposal: Update the `Selectable` to implement `FromSql` and `FromSqlRow` when a composite type is specified
mod grey_type_impls {
    use super::GreyType;
    use crate::schema::grey_type;
    use diesel::deserialize::{FromSql, FromSqlRow};
    use diesel::pg::Pg;
    use diesel::pg::PgValue;
    use diesel::row::Row;
    use diesel::sql_types::{CompositeType, Record};
    use diesel::{Expression, Queryable, Selectable};

    type GreyTypeTuple = <GreyType as Queryable<
        <<grey_type as CompositeType>::AllFields as Expression>::SqlType,
        Pg,
    >>::Row;

    impl From<GreyTypeTuple> for GreyType {
        fn from(value: GreyTypeTuple) -> Self {
            Self {
                intensity: value.0,
                suggestion: value.1,
            }
        }
    }

    impl FromSql<grey_type, Pg> for GreyType {
        fn from_sql(bytes: PgValue) -> diesel::deserialize::Result<Self> {
            let value: GreyTypeTuple = FromSql::<
                Record<<<grey_type as CompositeType>::AllFields as Expression>::SqlType>,
                Pg,
            >::from_sql(bytes)?;
            Ok(value.into())
        }
    }

    impl FromSqlRow<grey_type, Pg> for GreyType {
        fn build_from_row<'a>(row: &impl Row<'a, Pg>) -> diesel::deserialize::Result<Self> {
            let value: GreyTypeTuple = FromSqlRow::<
                Record<<<grey_type as CompositeType>::AllFields as Expression>::SqlType>,
                Pg,
            >::build_from_row(row)?;
            Ok(value.into())
        }
    }

    impl Selectable<Pg> for GreyType {
        type SelectExpression = <grey_type as CompositeType>::AllFields;

        fn construct_selection() -> Self::SelectExpression {
            <grey_type as CompositeType>::all_fields()
        }
    }
}

/// Module containing the diesel composite type, its field definitions, and required trait impls.
///
/// Proposal: Create a new macro similar to `table!`/`view!` to generate types and trait
///           implementations for composite types. This module contains the minimal implementations
///           required for my use case, but there may be more required for other use cases.
///           Depending on how much overlap there is with the `table!`/`view!` macro, we may be
///           able to re-use some of the existing `query_source_macro` implementation.
#[allow(non_camel_case_types, non_upper_case_globals)]
pub mod schema {
    use diesel::backend::Backend;
    use diesel::expression::ValidGrouping;
    use diesel::query_builder::{AstPass, QueryFragment};
    use diesel::query_source::Function;
    use diesel::sql_types::{CompositeType, Float, Text};
    use diesel::{AppearsOnTable, Expression, QueryId, QueryResult, SelectableExpression, SqlType};

    #[derive(Debug, Copy, Clone, SqlType)]
    #[diesel(postgres_type(name = "gray_type"))]
    pub struct grey_type;

    impl CompositeType for grey_type {
        type AllFields = AllFields;

        fn all_fields() -> Self::AllFields {
            all_fields
        }
    }

    #[derive(QueryId)]
    pub struct intensity;
    #[derive(QueryId)]
    pub struct suggestion;

    impl Expression for intensity {
        type SqlType = Float;
    }
    impl Expression for suggestion {
        type SqlType = Text;
    }

    impl<DB> QueryFragment<DB> for intensity
    where
        DB: Backend,
    {
        fn walk_ast<'b>(&'b self, mut pass: AstPass<'_, 'b, DB>) -> QueryResult<()> {
            pass.push_sql("\"intensity\"");
            Ok(())
        }
    }
    impl<DB> QueryFragment<DB> for suggestion
    where
        DB: Backend,
    {
        fn walk_ast<'b>(&'b self, mut pass: AstPass<'_, 'b, DB>) -> QueryResult<()> {
            pass.push_sql("\"suggestion\"");
            Ok(())
        }
    }

    impl ValidGrouping<()> for intensity {
        type IsAggregate = diesel::expression::is_aggregate::No;
    }
    impl ValidGrouping<()> for suggestion {
        type IsAggregate = diesel::expression::is_aggregate::No;
    }

    impl AppearsOnTable<grey_type> for intensity {}
    impl AppearsOnTable<grey_type> for suggestion {}

    impl SelectableExpression<grey_type> for intensity {}
    impl SelectableExpression<grey_type> for suggestion {}

    impl<F> AppearsOnTable<F> for intensity where F: Function<Return = grey_type> {}
    impl<F> AppearsOnTable<F> for suggestion where F: Function<Return = grey_type> {}

    impl<F> SelectableExpression<F> for intensity where F: Function<Return = grey_type> {}
    impl<F> SelectableExpression<F> for suggestion where F: Function<Return = grey_type> {}

    pub type AllFields = (intensity, suggestion);
    pub const all_fields: AllFields = (intensity, suggestion);
    pub type SqlType = <AllFields as Expression>::SqlType;
}

#[cfg(test)]
mod tests {
    use super::GreyType;
    use super::color2grey;
    use composite_types::schema::colors::dsl::colors;
    use composite_types::schema::colors::{blue, color_id, green, red};
    use diesel::pg::Pg;
    use diesel::{QueryDsl, SelectableHelper, debug_query, select};

    #[test]
    fn plain_invocation() {
        let query = select(color2grey(1, 2, 3));
        assert_eq!(
            "SELECT color2grey($1, $2, $3) -- binds: [1, 2, 3]",
            debug_query::<Pg, _>(&query).to_string()
        );
    }

    #[test]
    fn select_from() {
        let query = color2grey(1, 2, 3).select(GreyType::as_select());
        assert_eq!(
            "SELECT \"intensity\", \"suggestion\" FROM color2grey($1, $2, $3) -- binds: [1, 2, 3]",
            debug_query::<Pg, _>(&query).to_string()
        );
    }

    #[test]
    fn invoke_in_table_select() {
        let query = colors.select((color_id, color2grey(red, green, blue)));
        assert_eq!(
            "SELECT \"colors\".\"color_id\", color2grey(\"colors\".\"red\", \"colors\".\"green\", \"colors\".\"blue\") FROM \"colors\" -- binds: []",
            debug_query::<Pg, _>(&query).to_string()
        );
    }
}

fn main() {
    let connection = &mut establish_connection();
    // Experiment 1: Simply invoke `color2grey`
    let result: GreyType = select(color2grey(1, 2, 3))
        .get_result(connection)
        .expect("Error loading gray conversion");
    println!(
        "Intensity level {:?} with suggested name {:?}",
        result.intensity, result.suggestion
    );

    // Experiment 2: Invoke `color2grey`, selecting all fields
    let result: GreyType = color2grey(1, 2, 3)
        .select(GreyType::as_select())
        .get_result(connection)
        .expect("Error loading gray conversion");
    println!(
        "Intensity level {:?} with suggested name {:?}",
        result.intensity, result.suggestion
    );

    // Experiment 3: Invoke `color2grey`, only selecting the `intensity` field
    let calculated_intensity: f32 = color2grey(1, 2, 3)
        .select(intensity)
        .get_result(connection)
        .expect("Error loading gray conversion");
    println!("Intensity level {:?}", calculated_intensity);
}
