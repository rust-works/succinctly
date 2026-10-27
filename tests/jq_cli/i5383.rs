//! #5383: `strptime` derives the month and day from `%j` once a year was read, and rejects day 0 and 367+.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// The libc fills `tm_mon`/`tm_mday` from `tm_yday` when a year was read and nothing set the month or day (a day
// past the end of the year rolls into the next), and refuses a day of year outside 1-366. `%j` next to `%m`/`%d`/`%e`
// /`%D`/`%F`, or with no year, is order- and platform-dependent in the libc and some rows abort jq, so those stay as
// read. Every row captured from `/usr/bin/jq` 1.7.1 (stdout, error message, exit code), both routes.

/// #5383: the day of year with a year read (`%Y`, `%y`, `%C`), in either order, in leap and common years.
#[test]
fn test_5383_strptime_day_of_year_sets_month_and_day() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            "\"2015 060\" | strptime(\"%Y %j\")",
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 001\" | strptime(\"%Y %j\")",
            "[2015,0,1,0,0,0,4,0]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 031\" | strptime(\"%Y %j\")",
            "[2015,0,31,0,0,0,6,30]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 032\" | strptime(\"%Y %j\")",
            "[2015,1,1,0,0,0,0,31]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 365\" | strptime(\"%Y %j\")",
            "[2015,11,31,0,0,0,4,364]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 366\" | strptime(\"%Y %j\")",
            "[2016,0,1,0,0,0,5,0]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2016 366\" | strptime(\"%Y %j\")",
            "[2016,11,31,0,0,0,6,365]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2016 060\" | strptime(\"%Y %j\")",
            "[2016,1,29,0,0,0,1,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2000 366\" | strptime(\"%Y %j\")",
            "[2000,11,31,0,0,0,0,365]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2100 366\" | strptime(\"%Y %j\")",
            "[2101,0,1,0,0,0,6,0]\n",
            "",
            0,
        ),
        (
            "null",
            "\"1900 060\" | strptime(\"%Y %j\")",
            "[1900,2,1,0,0,0,4,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"060 2015\" | strptime(\"%j %Y\")",
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"15 060\" | strptime(\"%y %j\")",
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015-060\" | strptime(\"%Y-%j\")",
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"19 060\" | strptime(\"%C %j\")",
            "[1900,2,1,0,0,0,4,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"1999 060\" | strptime(\"%C%y %j\")",
            "[1999,2,1,0,0,0,1,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 060 10:30\" | strptime(\"%Y %j %H:%M\")",
            "[2015,2,1,10,30,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 60\" | strptime(\"%Y %j\")",
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            "\"2015 000\" | strptime(\"%Y %j\")",
            "",
            "date \"2015 000\" does not match format \"%Y %j\"",
            5,
        ),
        (
            "null",
            "\"2015 367\" | strptime(\"%Y %j\")",
            "",
            "date \"2015 367\" does not match format \"%Y %j\"",
            5,
        ),
        (
            "null",
            "\"2015 1000\" | strptime(\"%Y %j\")",
            "",
            "date \"2015 1000\" does not match format \"%Y %j\"",
            5,
        ),
        (
            "null",
            "\"000\" | strptime(\"%j\")",
            "",
            "date \"000\" does not match format \"%j\"",
            5,
        ),
        (
            "null",
            "\"367\" | strptime(\"%j\")",
            "",
            "date \"367\" does not match format \"%j\"",
            5,
        ),
    ])
}
