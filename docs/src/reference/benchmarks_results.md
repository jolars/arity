<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"small","formatter":"arity","mean_ms":25.0912,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.6294,"min_ms":21.0695,"max_ms":32.0858},{"document":"small","formatter":"air","mean_ms":31.632,"ratio":1.2606810355821962,"ratio_label":"1.3x slower","stddev_ms":2.1308,"min_ms":28.2261,"max_ms":41.2459},{"document":"large","formatter":"arity","mean_ms":761.727,"ratio":1.0,"ratio_label":"baseline","stddev_ms":50.2538,"min_ms":713.3926,"max_ms":815.4131},{"document":"large","formatter":"air","mean_ms":368.2495,"ratio":0.48344026140598934,"ratio_label":"2.1x faster","stddev_ms":12.4938,"min_ms":359.3317,"max_ms":393.7535}]</script>
<figcaption>Formatting speed relative to <code>arity</code>. Each dot is one corpus tier formatted by one tool; the vertical position is mean wall-clock time as a ratio to <code>arity</code> on a log scale, so <code>arity</code> lies on the dashed baseline at 1, faster tools fall below it and slower tools rise above. Color distinguishes tiers; hover a dot for the exact millisecond figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h3>small (123094 bytes, 8498 lines)</h3>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>25.0912</td><td>21.0695</td><td>32.0858</td><td>baseline</td></tr>
<tr><td>air</td><td>31.6320</td><td>28.2261</td><td>41.2459</td><td>1.3x slower</td></tr>
</tbody>
</table>
<h3>large (1477128 bytes, 101976 lines)</h3>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>761.7270</td><td>713.3926</td><td>815.4131</td><td>baseline</td></tr>
<tr><td>air</td><td>368.2495</td><td>359.3317</td><td>393.7535</td><td>2.1x faster</td></tr>
</tbody>
</table>
</details>
</div>
