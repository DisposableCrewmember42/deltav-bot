macro_rules! id_to_int {
    ($e: ident) => {
        let $e = $e.get().cast_signed();
    };
    ($e: ident, $($es:ident),+) => {
        id_to_int!($e);
        id_to_int!($($es),+);
    };
}

pub(crate) use id_to_int;
