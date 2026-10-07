use super::FunctionMacro;
use super::expand_with;

#[test]
pub(crate) fn composite_type_1() {
    let input = quote::quote! {
        test_type {
            a -> Integer,
            b -> Text,
        }
    };
    // Todo: Do other DBs support composite types?
    let name = if cfg!(feature = "postgres") {
        "composite_type_1 (postgres)"
    } else {
        "composite_type_1"
    };

    expand_with(
        &crate::composite_type_proc_inner as &dyn Fn(_) -> _,
        input,
        FunctionMacro(syn::parse_quote!(composite_type)),
        name,
    );
}
