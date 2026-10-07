use crate::table::{
    AggregateTokens, cfg_attributes, collect_cfg_groups, fix_import_for_submodule,
    generate_aggregate_variants, generate_op_impl, has_cfg_attributes, is_date_time, is_network,
    is_numeric,
};
use diesel_table_macro_syntax::{ColumnDef, ViewDecl};
use proc_macro2::{Ident, Span, TokenStream};
use syn::parse_quote;

const KIND_NAME: &str = "composite_type";

pub fn composite_type_macro(tokenstream2: TokenStream) -> TokenStream {
    // include the input in the error output so that rust-analyzer is happy
    match syn::parse2::<ViewDecl>(tokenstream2.clone()) {
        Ok(input) => expand(input),
        Err(_) => {
            quote::quote! {
                compile_error!(
                    concat!("invalid `", #KIND_NAME, "!` syntax \nhelp: please see the `", #KIND_NAME, "!` macro docs for more info\n\
                             help: docs available at: `https://docs.diesel.rs/", env!("CARGO_PKG_VERSION_MAJOR"), ".", env!("CARGO_PKG_VERSION_MINOR"), ".x/diesel/macro.", #KIND_NAME, ".html`\n"
                    ));
                #tokenstream2
            }
        }
    }
}

fn expand(input: ViewDecl) -> TokenStream {
    let column_count = input.column_defs.len() as u16;
    let too_many_columns_error_message = format!(
        "`{}` contains {column_count} columns, which is more than the supported maximum number of columns\n\
        Try enabling a crate level feature to support more columns",
        input.table_name
    );
    let meta = &input.meta;
    let table_name = &input.table_name;
    let imports = if input.use_statements.is_empty() {
        vec![parse_quote!(
            use diesel::sql_types::*;
        )]
    } else {
        input.use_statements.clone()
    };

    let non_gated_columns: Vec<_> = input
        .column_defs
        .iter()
        .filter(|c| !has_cfg_attributes(c))
        .collect();
    let cfg_groups = collect_cfg_groups(&input.column_defs);

    let query_source_ident = syn::Ident::new(KIND_NAME, input.table_name.span());

    let column_defs = input
        .column_defs
        .iter()
        .map(|c| expand_column_def(c, &query_source_ident));
    let valid_grouping_for_table_columns = generate_valid_grouping_for_table_columns(&input);

    let sql_name = &input.sql_name;
    let static_query_fragment_impl_for_table = if let Some(schema) = input.schema.as_ref() {
        let schema_name = schema.to_string();
        quote::quote! {
            impl diesel::internal::table_macro::StaticQueryFragment for #query_source_ident {
                type Component = diesel::internal::table_macro::InfixNode<
                        diesel::internal::table_macro::Identifier<'static>,
                    diesel::internal::table_macro::Identifier<'static>,
                    &'static str
                        >;
                const STATIC_COMPONENT: &'static Self::Component = &diesel::internal::table_macro::InfixNode::new(
                    diesel::internal::table_macro::Identifier(#schema_name),
                    diesel::internal::table_macro::Identifier(#sql_name),
                    "."
                );
            }
        }
    } else {
        quote::quote! {
            impl diesel::internal::table_macro::StaticQueryFragment for #query_source_ident {
                type Component = diesel::internal::table_macro::Identifier<'static>;
                const STATIC_COMPONENT: &'static Self::Component = &diesel::internal::table_macro::Identifier(#sql_name);
            }
        }
    };

    let reexport_column_from_dsl = input.column_defs.iter().map(|c| {
        let column_name = &c.column_name;
        let cfg_attrs = cfg_attributes(&c.meta);
        if c.column_name == *table_name {
            let span = Span::mixed_site().located_at(c.column_name.span());
            let message = format!(
                "column `{column_name}` cannot be named the same as it's {KIND_NAME}.\n\
                 you may use `#[sql_name = \"{column_name}\"]` to reference the {KIND_NAME}'s \
                 `{column_name}` column \n\
                 docs available at: `https://docs.diesel.rs/{}.x/diesel/macro.{KIND_NAME}.html`\n",
                env!("CARGO_PKG_VERSION")
                    .rsplit_once('.')
                    .expect("This is a valid version")
                    .0
            );
            quote::quote_spanned! { span =>
                compile_error!(#message);
            }
        } else {
            quote::quote! {
                #(#cfg_attrs)*
                pub use super::columns::#column_name;
            }
        }
    });

    let AggregateTokens {
        all_columns_const,
        all_columns_type_variants,
    } = generate_aggregate_variants(&non_gated_columns, &cfg_groups, KIND_NAME);

    let kind_specific_impls = quote::quote! {
        // #[doc(hidden)]
        // pub use self::composite_type as table;

        impl diesel::internal::table_macro::Sealed for composite_type {}

        impl CompositeType for composite_type {
            type AllFields = AllColumns;

            fn all_fields() -> Self::AllFields {
                all_columns
            }
        }

    };

    let marked = input.column_defs.iter().find(|c| c.auto_increment);
    if let Some(column) = marked {
        let span = Span::mixed_site().located_at(column.column_name.span());
        return quote::quote_spanned! {span=>
            compile_error!("`#[auto_increment]` is not supported in `composite_type!` definitions");
        };
    }

    let imports_for_column_module = imports.iter().map(fix_import_for_submodule);

    let diesel_pg_type = if let Some(schema) = input.schema.as_ref() {
        let schema_name = schema.to_string();
        quote::quote! {
            #[diesel(postgres_type(name = #sql_name, schema = #schema_name))]
        }
    } else {
        quote::quote! {
            #[diesel(postgres_type(name = #sql_name))]
        }
    };

    quote::quote! {
        #(#meta)*
        #[allow(unused_imports, dead_code, unreachable_pub, unused_qualifications)]
        pub mod #table_name {
            const _: () = {
                assert!(
                    #column_count <= diesel::internal::table_macro::MAX_COLUMN_COUNT,
                    #too_many_columns_error_message
                );
            };

            use ::diesel;
            pub use self::columns::*;
            #(#imports)*

            #[doc = concat!("Re-exports all of the columns of this ", #KIND_NAME, ", as well as the")]
            #[doc = concat!(#KIND_NAME, " struct renamed to the module name. This is meant to be")]
            #[doc = concat!("glob imported for functions which only deal with one ", #KIND_NAME, ".")]
            pub mod dsl {
                #(#reexport_column_from_dsl)*
                pub use super::#query_source_ident as #table_name;
            }

            #all_columns_const

            #[allow(non_camel_case_types)]
            #[derive(Debug, Clone, Copy, diesel::query_builder::QueryId, Default, PartialEq, Eq, PartialOrd, Ord, Hash, diesel::sql_types::SqlType)]
            #diesel_pg_type
            #[doc = concat!("The actual ", #KIND_NAME, " struct")]
            ///
            /// This is the type which provides the base methods of the query
            /// builder, such as `.select` and `.filter`.
            pub struct #query_source_ident;
            //
            // impl #query_source_ident {
            //     // #[allow(dead_code)]
            //     // #[doc = concat!("Represents `", #KIND_NAME, "_name.*`, which is sometimes necessary")]
            //     // /// for efficient count queries. It cannot be used in place of
            //     // /// `all_columns`
            //     // pub fn star(&self) -> star {
            //     //     star
            //     // }
            // }
            //
            #all_columns_type_variants

            #[doc = concat!("The SQL type of all of the columns on this ", #KIND_NAME)]
            pub type SqlType = <AllColumns as diesel::Expression>::SqlType;

            // #[doc = concat!("Helper type for representing a boxed query from this ", #KIND_NAME)]
            // pub type BoxedQuery<'a, DB, ST = SqlType> = diesel::internal::table_macro::BoxedSelectStatement<'a, ST, diesel::internal::table_macro::FromClause<#query_source_ident>, DB>;

            // #[doc = concat!("Helper type for representing a boxed cloneable query from this ", #KIND_NAME)]
            // pub type BoxedCloneQuery<'a, DB, ST = SqlType> = diesel::internal::table_macro::BoxedCloneSelectStatement<'a, ST, diesel::internal::table_macro::FromClause<#query_source_ident>, DB>;


            // impl diesel::QuerySource for #query_source_ident {
            //     type FromClause = diesel::internal::table_macro::StaticQueryFragmentInstance<#query_source_ident>;
            //     type DefaultSelection = <Self as diesel::query_source::QueryRelation>::AllColumns;
            //
            //     fn from_clause(&self) -> Self::FromClause {
            //         diesel::internal::table_macro::StaticQueryFragmentInstance::new()
            //     }
            //
            //     fn default_selection(&self) -> Self::DefaultSelection {
            //         <Self as diesel::query_source::QueryRelation>::all_columns()
            //     }
            // }

            // impl diesel::internal::table_macro::PlainQuerySource for #query_source_ident {}

            impl<DB> diesel::query_builder::QueryFragment<DB> for #query_source_ident where
                DB: diesel::backend::Backend,
                <Self as diesel::internal::table_macro::StaticQueryFragment>::Component: diesel::query_builder::QueryFragment<DB>
            {
                fn walk_ast<'b>(&'b self, __diesel_internal_pass: diesel::query_builder::AstPass<'_, 'b, DB>) -> diesel::result::QueryResult<()> {
                    <Self as diesel::internal::table_macro::StaticQueryFragment>::STATIC_COMPONENT.walk_ast(__diesel_internal_pass)
                }
            }

            #static_query_fragment_impl_for_table

            // impl diesel::query_builder::AsQuery for #query_source_ident {
            //     type SqlType = SqlType;
            //     type Query = diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<Self>>;
            //
            //     fn as_query(self) -> Self::Query {
            //         diesel::internal::table_macro::SelectStatement::simple(self)
            //     }
            // }

            #kind_specific_impls

            // #auto_increment_impl

            // Todo: Implement `AppearsInFromClause`? The `Count` would always be `Never`, so it might be a useless impl.
            // impl diesel::query_source::AppearsInFromClause<Self> for #query_source_ident {
            //     type Count = diesel::query_source::Never;
            // }
            //
            // // impl<S: AliasSource<Table=table>> AppearsInFromClause<table> for Alias<S>
            // impl<S> diesel::internal::table_macro::AliasAppearsInFromClause<S, Self> for #query_source_ident
            // where S: diesel::query_source::AliasSource<Target = Self>,
            // {
            //     type Count = diesel::query_source::Never;
            // }
            //
            // // impl<S1: AliasSource<Table=table>, S2: AliasSource<Table=table>> AppearsInFromClause<Alias<S1>> for Alias<S2>
            // // Those are specified by the `alias!` macro, but this impl will allow it to implement this trait even in downstream
            // // crates from the schema
            // impl<S1, S2> diesel::internal::table_macro::AliasAliasAppearsInFromClause<Self, S2, S1> for #query_source_ident
            // where S1: diesel::query_source::AliasSource<Target = Self>,
            //       S2: diesel::query_source::AliasSource<Target = Self>,
            //       S1: diesel::internal::table_macro::AliasAliasAppearsInFromClauseSameTable<S2, Self>,
            // {
            //     type Count = <S1 as diesel::internal::table_macro::AliasAliasAppearsInFromClauseSameTable<S2, Self>>::Count;
            // }
            //
            // impl<S> diesel::query_source::AppearsInFromClause<diesel::query_source::Alias<S>> for #query_source_ident
            // where S: diesel::query_source::AliasSource,
            // {
            //     type Count = diesel::query_source::Never;
            // }
            //
            // impl<S, C> diesel::internal::table_macro::FieldAliasMapperAssociatedTypesDisjointnessTrick<Self, S, C> for #query_source_ident
            // where
            //     S: diesel::query_source::AliasSource<Target = Self> + ::core::clone::Clone,
            //     C: diesel::query_source::QueryRelationField<QueryRelation = Self>,
            // {
            //     type Out = diesel::query_source::AliasedField<S, C>;
            //
            //     fn map(__diesel_internal_column: C, __diesel_internal_alias: &diesel::query_source::Alias<S>) -> Self::Out {
            //         __diesel_internal_alias.field(__diesel_internal_column)
            //     }
            // }
            //
            // impl<StmtKind> diesel::query_source::AppearsInFromClause<#query_source_ident> for diesel::internal::table_macro::returning::ReturningQuerySource<StmtKind, #query_source_ident>
            // {
            //     type Count = diesel::query_source::Never;
            // }
            //
            // impl<StmtKind, T> diesel::query_source::AppearsInFromClause<diesel::internal::table_macro::returning::ReturningQuerySource<StmtKind, T>> for #query_source_ident {
            //     type Count = diesel::query_source::Never;
            // }
            //
            // impl diesel::query_source::AppearsInFromClause<#query_source_ident> for diesel::internal::table_macro::NoFromClause {
            //     type Count = diesel::query_source::Never;
            // }

            // Todo: Allow joins?
            // impl<Left, Right, Kind> diesel::JoinTo<diesel::internal::table_macro::Join<Left, Right, Kind>> for #query_source_ident where
            //     diesel::internal::table_macro::Join<Left, Right, Kind>: diesel::JoinTo<Self>,
            //     Left: diesel::query_source::QuerySource,
            //     Right: diesel::query_source::QuerySource,
            // {
            //     type FromClause = diesel::internal::table_macro::Join<Left, Right, Kind>;
            //     type OnClause = <diesel::internal::table_macro::Join<Left, Right, Kind> as diesel::JoinTo<Self>>::OnClause;
            //
            //     fn join_target(__diesel_internal_rhs: diesel::internal::table_macro::Join<Left, Right, Kind>) -> (Self::FromClause, Self::OnClause) {
            //         let (_, __diesel_internal_on_clause) = diesel::internal::table_macro::Join::join_target(Self);
            //         (__diesel_internal_rhs, __diesel_internal_on_clause)
            //     }
            // }
            //
            // impl<Join, On> diesel::JoinTo<diesel::internal::table_macro::JoinOn<Join, On>> for #query_source_ident where
            //     diesel::internal::table_macro::JoinOn<Join, On>: diesel::JoinTo<Self>,
            // {
            //     type FromClause = diesel::internal::table_macro::JoinOn<Join, On>;
            //     type OnClause = <diesel::internal::table_macro::JoinOn<Join, On> as diesel::JoinTo<Self>>::OnClause;
            //
            //     fn join_target(__diesel_internal_rhs: diesel::internal::table_macro::JoinOn<Join, On>) -> (Self::FromClause, Self::OnClause) {
            //         let (_, __diesel_internal_on_clause) = diesel::internal::table_macro::JoinOn::join_target(Self);
            //         (__diesel_internal_rhs, __diesel_internal_on_clause)
            //     }
            // }
            //
            // impl<F, S, D, W, O, L, Of, G> diesel::JoinTo<diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<F>, S, D, W, O, L, Of, G>> for #query_source_ident where
            //     diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<F>, S, D, W, O, L, Of, G>: diesel::JoinTo<Self>,
            //     F: diesel::query_source::QuerySource
            // {
            //     type FromClause = diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<F>, S, D, W, O, L, Of, G>;
            //     type OnClause = <diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<F>, S, D, W, O, L, Of, G> as diesel::JoinTo<Self>>::OnClause;
            //
            //     fn join_target(__diesel_internal_rhs: diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<F>, S, D, W, O, L, Of, G>) -> (Self::FromClause, Self::OnClause) {
            //         let (_, __diesel_internal_on_clause) = diesel::internal::table_macro::SelectStatement::join_target(Self);
            //         (__diesel_internal_rhs, __diesel_internal_on_clause)
            //     }
            // }
            //
            // impl<'a, QS, ST, DB> diesel::JoinTo<diesel::internal::table_macro::BoxedSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>> for #query_source_ident where
            //     diesel::internal::table_macro::BoxedSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>: diesel::JoinTo<Self>,
            //     QS: diesel::query_source::QuerySource,
            // {
            //     type FromClause = diesel::internal::table_macro::BoxedSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>;
            //     type OnClause = <diesel::internal::table_macro::BoxedSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB> as diesel::JoinTo<Self>>::OnClause;
            //     fn join_target(__diesel_internal_rhs: diesel::internal::table_macro::BoxedSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>) -> (Self::FromClause, Self::OnClause) {
            //         let (_, __diesel_internal_on_clause) = diesel::internal::table_macro::BoxedSelectStatement::join_target(Self);
            //         (__diesel_internal_rhs, __diesel_internal_on_clause)
            //     }
            // }
            //
            // impl<'a, QS, ST, DB> diesel::JoinTo<diesel::internal::table_macro::BoxedCloneSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>> for #query_source_ident where
            //     diesel::internal::table_macro::BoxedCloneSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>: diesel::JoinTo<Self>,
            //     QS: diesel::query_source::QuerySource,
            // {
            //     type FromClause = diesel::internal::table_macro::BoxedCloneSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>;
            //     type OnClause = <diesel::internal::table_macro::BoxedCloneSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB> as diesel::JoinTo<Self>>::OnClause;
            //     fn join_target(__diesel_internal_rhs: diesel::internal::table_macro::BoxedCloneSelectStatement<'a, diesel::internal::table_macro::FromClause<QS>, ST, DB>) -> (Self::FromClause, Self::OnClause) {
            //         let (_, __diesel_internal_on_clause) = diesel::internal::table_macro::BoxedCloneSelectStatement::join_target(Self);
            //         (__diesel_internal_rhs, __diesel_internal_on_clause)
            //     }
            // }
            //
            // impl<S> diesel::JoinTo<diesel::query_source::Alias<S>> for #query_source_ident
            // where
            //     diesel::query_source::Alias<S>: diesel::JoinTo<Self>,
            // {
            //     type FromClause = diesel::query_source::Alias<S>;
            //     type OnClause = <diesel::query_source::Alias<S> as diesel::JoinTo<Self>>::OnClause;
            //
            //     fn join_target(__diesel_internal_rhs: diesel::query_source::Alias<S>) -> (Self::FromClause, Self::OnClause) {
            //         let (_, __diesel_internal_on_clause) = diesel::query_source::Alias::<S>::join_target(Self);
            //         (__diesel_internal_rhs, __diesel_internal_on_clause)
            //     }
            // }

            #[doc = concat!("Contains all of the columns of this ", #KIND_NAME)]
            pub mod columns {
                use ::diesel;
                use super::#query_source_ident;
                #(#imports_for_column_module)*

                // Todo: Add `star` impl?
                // #[allow(non_camel_case_types, dead_code)]
                // #[derive(Debug, Clone, Copy, diesel::query_builder::QueryId, PartialEq, Eq, PartialOrd, Ord, Hash)]
                // #[doc = concat!("Represents `", #KIND_NAME, "_name.*`, which is sometimes needed for")]
                // /// efficient count queries. It cannot be used in place of
                // /// `all_columns`, and has a `SqlType` of `()` to prevent it
                // /// being used that way
                // pub struct star;
                //
                // impl<__GB> diesel::expression::ValidGrouping<__GB> for star
                // where
                //     super::AllColumns: diesel::expression::ValidGrouping<__GB>,
                // {
                //     type IsAggregate =
                //         <super::AllColumns as diesel::expression::ValidGrouping<__GB>>::IsAggregate;
                // }
                //
                // impl diesel::Expression for star {
                //     type SqlType = diesel::expression::expression_types::NotSelectable;
                // }
                //
                // impl<DB: diesel::backend::Backend> diesel::query_builder::QueryFragment<DB> for star where
                //     <#query_source_ident as diesel::QuerySource>::FromClause: diesel::query_builder::QueryFragment<DB>,
                // {
                //     #[allow(non_snake_case)]
                //     fn walk_ast<'b>(&'b self, mut __diesel_internal_out: diesel::query_builder::AstPass<'_, 'b, DB>) -> diesel::result::QueryResult<()>
                //     {
                //         use diesel::QuerySource;
                //
                //         if !__diesel_internal_out.should_skip_from() {
                //             const FROM_CLAUSE: diesel::internal::table_macro::StaticQueryFragmentInstance<#query_source_ident> = diesel::internal::table_macro::StaticQueryFragmentInstance::new();
                //
                //             FROM_CLAUSE.walk_ast(__diesel_internal_out.reborrow())?;
                //             __diesel_internal_out.push_sql(".");
                //         }
                //         __diesel_internal_out.push_sql("*");
                //         Ok(())
                //     }
                // }
                //
                // impl diesel::SelectableExpression<#query_source_ident> for star {}
                //
                // impl diesel::AppearsOnTable<#query_source_ident> for star {}

                #(#column_defs)*

                #(#valid_grouping_for_table_columns)*
            }
        }
    }
}

fn expand_column_def(column_def: &ColumnDef, query_source_ident: &Ident) -> TokenStream {
    // TODO get a better span here as soon as that's
    // possible using stable rust
    let span = Span::mixed_site().located_at(column_def.column_name.span());
    let meta = &column_def.meta;
    let cfg_attrs = cfg_attributes(&column_def.meta);
    let column_name = &column_def.column_name;
    let sql_name = &column_def.sql_name;
    let sql_type = &column_def.tpe;

    let ops_impls = if is_numeric(&column_def.tpe) {
        let add = generate_op_impl("Add", column_name, &cfg_attrs);
        let sub = generate_op_impl("Sub", column_name, &cfg_attrs);
        let div = generate_op_impl("Div", column_name, &cfg_attrs);
        let mul = generate_op_impl("Mul", column_name, &cfg_attrs);
        Some(quote::quote! {
            #add
            #sub
            #div
            #mul
        })
    } else if is_date_time(&column_def.tpe) || is_network(&column_def.tpe) {
        let add = generate_op_impl("Add", column_name, &cfg_attrs);
        let sub = generate_op_impl("Sub", column_name, &cfg_attrs);
        Some(quote::quote! {
            #add
            #sub
        })
    } else {
        None
    };

    let max_length = column_def.max_length.as_ref().map(|column_max_length| {
        quote::quote! {
            #(#cfg_attrs)*
            impl self::diesel::query_source::SizeRestrictedColumn for #column_name {
                const MAX_LENGTH: usize = #column_max_length;
            }
        }
    });

    quote::quote_spanned! {span=>
        #(#meta)*
        #[allow(non_camel_case_types, dead_code)]
        #[derive(Debug, Clone, Copy, diesel::query_builder::QueryId, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct #column_name;

        #(#cfg_attrs)*
        impl diesel::expression::Expression for #column_name {
            type SqlType = #sql_type;
        }

        #(#cfg_attrs)*
        impl<DB> diesel::query_builder::QueryFragment<DB> for #column_name where
            DB: diesel::backend::Backend,
            diesel::internal::table_macro::StaticQueryFragmentInstance<#query_source_ident>: diesel::query_builder::QueryFragment<DB>,
        {
            #[allow(non_snake_case)]
            fn walk_ast<'b>(&'b self, mut __diesel_internal_out: diesel::query_builder::AstPass<'_, 'b, DB>) -> diesel::result::QueryResult<()>
            {
                if !__diesel_internal_out.should_skip_from() {
                    const FROM_CLAUSE: diesel::internal::table_macro::StaticQueryFragmentInstance<#query_source_ident> = diesel::internal::table_macro::StaticQueryFragmentInstance::new();

                    FROM_CLAUSE.walk_ast(__diesel_internal_out.reborrow())?;
                    __diesel_internal_out.push_sql(".");
                }
                __diesel_internal_out.push_identifier(#sql_name)
            }
        }

        #(#cfg_attrs)*
        impl diesel::SelectableExpression<super::#query_source_ident> for #column_name {
        }

        #(#cfg_attrs)*
        impl diesel::AppearsOnTable<super::#query_source_ident> for #column_name {
        }
        //
        // #(#cfg_attrs)*
        // impl<__StmtKind>
        //     diesel::SelectableExpression<
        //         diesel::internal::table_macro::returning::ReturningQuerySource<
        //             __StmtKind,
        //             super::#query_source_ident,
        //         >,
        //     > for #column_name
        // {
        // }
        //
        // #(#cfg_attrs)*
        // impl<QS> diesel::AppearsOnTable<QS> for #column_name where
        //     QS: diesel::query_source::AppearsInFromClause<super::#query_source_ident, Count=diesel::query_source::Once>,
        // {
        // }

        //
        // #(#cfg_attrs)*
        // impl<Left, Right> diesel::SelectableExpression<
        //         diesel::internal::table_macro::Join<Left, Right, diesel::internal::table_macro::LeftOuter>,
        //     > for #column_name where
        //     #column_name: diesel::AppearsOnTable<diesel::internal::table_macro::Join<Left, Right, diesel::internal::table_macro::LeftOuter>>,
        //     Self: diesel::SelectableExpression<Left>,
        //     // If our table is on the right side of this join, only
        //     // `Nullable<Self>` can be selected
        //     Right: diesel::query_source::AppearsInFromClause<super::#query_source_ident, Count=diesel::query_source::Never> + diesel::query_source::QuerySource,
        //     Left: diesel::query_source::QuerySource
        // {
        // }
        //
        // #(#cfg_attrs)*
        // impl<Left, Right> diesel::SelectableExpression<
        //         diesel::internal::table_macro::Join<Left, Right, diesel::internal::table_macro::Inner>,
        //     > for #column_name where
        //     #column_name: diesel::AppearsOnTable<diesel::internal::table_macro::Join<Left, Right, diesel::internal::table_macro::Inner>>,
        //     Left: diesel::query_source::AppearsInFromClause<super::#query_source_ident> + diesel::query_source::QuerySource,
        //     Right: diesel::query_source::AppearsInFromClause<super::#query_source_ident> + diesel::query_source::QuerySource,
        // (Left::Count, Right::Count): diesel::internal::table_macro::Pick<Left, Right>,
        //     Self: diesel::SelectableExpression<
        //         <(Left::Count, Right::Count) as diesel::internal::table_macro::Pick<Left, Right>>::Selection,
        //     >,
        // {
        // }
        //
        // // FIXME: Remove this when overlapping marker traits are stable
        // #(#cfg_attrs)*
        // impl<Join, On> diesel::SelectableExpression<diesel::internal::table_macro::JoinOn<Join, On>> for #column_name where
        //     #column_name: diesel::SelectableExpression<Join> + diesel::AppearsOnTable<diesel::internal::table_macro::JoinOn<Join, On>>,
        // {
        // }
        //
        // // FIXME: Remove this when overlapping marker traits are stable
        // #(#cfg_attrs)*
        // impl<From> diesel::SelectableExpression<diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<From>>> for #column_name where
        //     From: diesel::query_source::QuerySource,
        //     #column_name: diesel::SelectableExpression<From> + diesel::AppearsOnTable<diesel::internal::table_macro::SelectStatement<diesel::internal::table_macro::FromClause<From>>>,
        // {
        // }
        //
        // #(#cfg_attrs)*
        // impl<__GB> diesel::expression::ValidGrouping<__GB> for #column_name
        // where __GB: diesel::expression::IsContainedInGroupBy<#column_name, Output = diesel::expression::is_contained_in_group_by::Yes>,
        // {
        //     type IsAggregate = diesel::expression::is_aggregate::Yes;
        // }
        //
        #(#cfg_attrs)*
        impl diesel::expression::ValidGrouping<()> for #column_name {
            type IsAggregate = diesel::expression::is_aggregate::No;
        }
        //
        // #(#cfg_attrs)*
        // impl<__GB, __F> diesel::expression::ValidGrouping<diesel::internal::table_macro::SubselectGroupBy<__GB, __F>> for #column_name
        // where
        //     __F: diesel::query_source::AppearsInFromClause<super::#query_source_ident>,
        //     __F::Count: diesel::internal::table_macro::SubselectFieldGrouping<Self, __GB>,
        // {
        //     type IsAggregate = <__F::Count as diesel::internal::table_macro::SubselectFieldGrouping<Self, __GB>>::IsAggregate;
        // }
        //
        // #(#cfg_attrs)*
        // impl diesel::expression::IsContainedInGroupBy<#column_name> for #column_name {
        //     type Output = diesel::expression::is_contained_in_group_by::Yes;
        // }
        //
        //
        //
        #(#cfg_attrs)*
        impl<T> diesel::EqAll<T> for #column_name where
            T: diesel::expression::AsExpression<#sql_type>,
            diesel::dsl::Eq<#column_name, T::Expression>: diesel::Expression<SqlType=diesel::sql_types::Bool>,
        {
            type Output = diesel::dsl::Eq<Self, T::Expression>;

            fn eq_all(self, __diesel_internal_rhs: T) -> Self::Output {
                use diesel::expression_methods::ExpressionMethods;
                self.eq(__diesel_internal_rhs)
            }
        }

        #max_length

        #ops_impls
    }
}

fn generate_valid_grouping_for_table_columns(view: &ViewDecl) -> Vec<TokenStream> {
    let mut ret = Vec::with_capacity(view.column_defs.len() * view.column_defs.len());

    for (id, right_col_def) in view.column_defs.iter().enumerate() {
        for left_col_def in view.column_defs.iter().skip(id) {
            // let right_to_left = if Some(left_col_def.column_name.to_string()) == primary_key {
            //     Ident::new("Yes", proc_macro2::Span::mixed_site())
            // } else {
            //     Ident::new("No", proc_macro2::Span::mixed_site())
            // };
            //
            // let left_to_right = if Some(right_col_def.column_name.to_string()) == primary_key {
            //     Ident::new("Yes", proc_macro2::Span::mixed_site())
            // } else {
            //     Ident::new("No", proc_macro2::Span::mixed_site())
            // };

            // Todo: Should these always be No or Yes?
            let right_to_left = Ident::new("No", proc_macro2::Span::mixed_site());
            let left_to_right = Ident::new("No", proc_macro2::Span::mixed_site());

            let left_col = &left_col_def.column_name;
            let right_col = &right_col_def.column_name;

            let left_cfg_attrs = cfg_attributes(&left_col_def.meta);
            let right_cfg_attrs = cfg_attributes(&right_col_def.meta);

            if left_col != right_col {
                ret.push(quote::quote! {
                    #(#left_cfg_attrs)*
                    #(#right_cfg_attrs)*
                    impl diesel::expression::IsContainedInGroupBy<#right_col> for #left_col {
                        type Output = diesel::expression::is_contained_in_group_by::#right_to_left;
                    }

                    #(#left_cfg_attrs)*
                    #(#right_cfg_attrs)*
                    impl diesel::expression::IsContainedInGroupBy<#left_col> for #right_col {
                        type Output = diesel::expression::is_contained_in_group_by::#left_to_right;
                    }
                });
            }
        }
    }
    ret
}
