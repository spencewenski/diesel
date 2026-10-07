mod schema {
    diesel::composite_type! {
        gray_type {
            id -> Integer,
            intensity -> Float,
            suggestion -> Text,
        }
    }
}

fn main() {}
