class.lim <- ## retain non-NA limitlists only
  lapply(limitlist[!all.na], class)
