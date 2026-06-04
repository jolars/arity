# Nested single-arg calls break the innermost arg list, hugging outer parens.
foo(bar(baz(qux(some_really_long_argument_name_that_will_overflow_the_line_yes))))

# A multi-arg inner breaks one-per-line; the trailing `)` counts toward the fit.
c(list(alpha_long, beta_long, gamma_long, delta_long, epsilon_long, zeta_longxx))

# `::` and `$` access binaries hug (space-less operator reads as the callee).
wrap(pkg::some_function(aaaaaaaaaa, bbbbbbbbbb, cccccccccc, dddddddddd, eeeeee))
wrap(obj$some_method(aaaaaaaaaa, bbbbbbbbbb, cccccccccc, dddddddddd, eeeeeeeeee))

# A spaced operator binary is not hugged: its argument drops to its own line.
filter(any(status_aaaaaaaaaaaaaaaaaaaa %in% c("hurricane", "tropical storm", "depr")))

# Everything fitting stays flat.
foo(bar(baz(qux(short))))
