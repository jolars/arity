# leads function
function() {}

# leads function
function() {}

function(
  # dangles ()
) {}

function(
  # dangles ()
) {}

function() {
  # dangles {}
}

function() a # trails function

function() {
  # leads `a`
  a
}

function() {
  # leads `a`
  # an inner comment
  a
}

function() {
  # dangles {}
}

function() {
  # dangles {}
}

function() {
  # dangles {}
  # an inner comment but empty `{}`
}

function() {
  # leads `a`
  a
}

# Not much we can do here, it's not enclosed by the `function_definition` node
# so it ends up trailing the `}` of the function. This is consistent with
# non-enclosed comments in if/else and loops.
function() a # trails function

function(
  # leads `a`
  a
) {
  # comment
}

function(
  a # trails `a`
) {
  # comment
}

function(
  a
  # trails `a`
) {
  # comment
}
