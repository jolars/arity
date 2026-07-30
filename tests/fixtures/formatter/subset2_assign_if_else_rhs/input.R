clean_columns = function(obj, factorsAsCharacter) {
	for (i in seq_along(obj)) {
		if (is.factor(obj[[i]])) {
			obj[[i]] = if (factorsAsCharacter)
					as.character(obj[[i]])
				else
					as.numeric(obj[[i]])
		}
	}
}
