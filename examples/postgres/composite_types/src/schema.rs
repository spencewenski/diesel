// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(Clone, diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "my_enum"))]
    #[diesel(enum_type)]
    pub struct MyEnum;

    diesel::composite_type! {
        use diesel::sql_types::*;
        use super::MyEnum;

        composite_with_enum_field {
            a -> Int4,
            b -> Nullable<Text>,
            my_enum_value -> Nullable<MyEnum>,
        }
    }

    #[allow(dead_code)]
    pub type CompositeWithEnumField = composite_with_enum_field::composite_type;

    diesel::composite_type! {
        gray_type {
            intensity -> Nullable<Float4>,
            suggestion -> Nullable<Text>,
        }
    }

    #[allow(dead_code)]
    pub type GrayType = gray_type::composite_type;

    diesel::composite_type! {
        my_comp_type {
            coord_id -> Nullable<Int4>,
            distance -> Nullable<Float4>,
        }
    }

    #[allow(dead_code)]
    pub type MyCompType = my_comp_type::composite_type;
}

pub mod rust_types {
    #[derive(Debug, PartialEq, diesel::types::Enum)]
    #[diesel(sql_type = super::sql_types::MyEnum)]
    pub enum MyEnum {
        #[diesel(rename = "A")]
        A,
        #[diesel(rename = "B")]
        B,
    }
}

diesel::table! {
    colors (color_id) {
        color_id -> Int4,
        red -> Int4,
        green -> Int4,
        blue -> Int4,
        color_name -> Nullable<Text>,
    }
}

diesel::table! {
    coordinates (coord_id) {
        coord_id -> Int4,
        xcoord -> Int4,
        ycoord -> Int4,
    }
}

diesel::allow_tables_to_appear_in_same_query!(colors, coordinates,);
