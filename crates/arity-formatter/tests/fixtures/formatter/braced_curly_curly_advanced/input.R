fn({{ var_that_is_extremely_long_and_eventually_forces_a_line_break_once_we_eventually_get_to_the_end }})

fn({{ # Leading of `var`
  var
}})

# Comprehensive comment test
fn(
# C1
{ # C2 (lifted up)
# C3 (lifted up)
{ # C4 (leads var)
  # C5 (leads var)
  var
  # C6
} # C7 (this line, but after folded 2nd `}`)
# C8 (after both `}}`)
} # C9 (same line as C8)
# C10
)
