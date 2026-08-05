# Inner single-bracket subset ending adjacent to outer single-bracket close.
# `]]` is lexed as one token but here it is two separate `]` closes.
df[df$col > 7, map[
  names(df)
]]

# Same shape followed by a comment block.
df[df$col > 7, map[
  names(df)
]]
# trailing comment

# Genuine subset2: `]]` legitimately closes `[[`.
x[[i]]

# Triple `]`: inner `[`, outer `[[`.
a[[b[c]]]

# Inner `[[`, outer `[`.
a[b[[c]]]
