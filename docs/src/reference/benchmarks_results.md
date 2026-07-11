### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"small","tool":"arity","mean_ms":32.0396,"ratio":1.0,"ratio_label":"baseline","stddev_ms":4.4454,"min_ms":24.9245,"max_ms":44.8228},{"document":"small","tool":"air","mean_ms":48.1975,"ratio":1.5043102910148691,"ratio_label":"1.5x slower","stddev_ms":9.61,"min_ms":30.7659,"max_ms":73.3525},{"document":"large","tool":"arity","mean_ms":1496.6197,"ratio":1.0,"ratio_label":"baseline","stddev_ms":403.0458,"min_ms":1036.8584,"max_ms":1789.0344},{"document":"large","tool":"air","mean_ms":500.2822,"ratio":0.3342747659943271,"ratio_label":"3.0x faster","stddev_ms":62.1606,"min_ms":414.6125,"max_ms":591.5527}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per synthetic corpus tier. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>small (123094 bytes, 8498 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>32.0396</td><td>24.9245</td><td>44.8228</td><td>baseline</td></tr>
<tr><td>air</td><td>48.1975</td><td>30.7659</td><td>73.3525</td><td>1.5x slower</td></tr>
</tbody>
</table>
<h5>large (1477128 bytes, 101976 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>1496.6197</td><td>1036.8584</td><td>1789.0344</td><td>baseline</td></tr>
<tr><td>air</td><td>500.2822</td><td>414.6125</td><td>591.5527</td><td>3.0x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":47.5794,"ratio":1.0,"ratio_label":"baseline","stddev_ms":5.5011,"min_ms":39.513,"max_ms":65.5785},{"document":"tidyr","tool":"air","mean_ms":55.2943,"ratio":1.1621479043451577,"ratio_label":"1.2x slower","stddev_ms":4.8034,"min_ms":47.7175,"max_ms":63.6572}]</script>
<figcaption>Formatting speed on a real R package (the tidyr source tree) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>47.5794</td><td>39.5130</td><td>65.5785</td><td>baseline</td></tr>
<tr><td>air</td><td>55.2943</td><td>47.7175</td><td>63.6572</td><td>1.2x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"small","tool":"arity","mean_ms":650.1199,"ratio":1.0,"ratio_label":"baseline","stddev_ms":45.7249,"min_ms":585.1538,"max_ms":686.1183},{"document":"small","tool":"jarl","mean_ms":39.2111,"ratio":0.060313643683265195,"ratio_label":"16.6x faster","stddev_ms":4.5893,"min_ms":32.7493,"max_ms":48.4486},{"document":"large","tool":"arity","mean_ms":88268.146,"ratio":1.0,"ratio_label":"baseline","stddev_ms":3247.7992,"min_ms":84522.222,"max_ms":90296.7985},{"document":"large","tool":"jarl","mean_ms":540.073,"ratio":0.006118549266912211,"ratio_label":"163.4x faster","stddev_ms":33.474,"min_ms":507.9284,"max_ms":595.3187}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per synthetic corpus tier, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>small (123094 bytes, 8498 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>650.1199</td><td>585.1538</td><td>686.1183</td><td>baseline</td></tr>
<tr><td>jarl</td><td>39.2111</td><td>32.7493</td><td>48.4486</td><td>16.6x faster</td></tr>
</tbody>
</table>
<h5>large (1477128 bytes, 101976 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>88268.1460</td><td>84522.2220</td><td>90296.7985</td><td>baseline</td></tr>
<tr><td>jarl</td><td>540.0730</td><td>507.9284</td><td>595.3187</td><td>163.4x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":80.5273,"ratio":1.0,"ratio_label":"baseline","stddev_ms":8.5004,"min_ms":68.5035,"max_ms":97.1317},{"document":"tidyr","tool":"jarl","mean_ms":20.49,"ratio":0.25444787047373996,"ratio_label":"3.9x faster","stddev_ms":2.1116,"min_ms":16.9229,"max_ms":27.4366}]</script>
<figcaption>Linting speed on a real R package (the tidyr source tree) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>80.5273</td><td>68.5035</td><td>97.1317</td><td>baseline</td></tr>
<tr><td>jarl</td><td>20.4900</td><td>16.9229</td><td>27.4366</td><td>3.9x faster</td></tr>
</tbody>
</table>
</details>
</div>

