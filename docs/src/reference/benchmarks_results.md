### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"small","tool":"arity","mean_ms":26.3464,"ratio":1.0,"ratio_label":"baseline","stddev_ms":3.1223,"min_ms":20.5507,"max_ms":36.0864},{"document":"small","tool":"air","mean_ms":35.968,"ratio":1.365196004008138,"ratio_label":"1.4x slower","stddev_ms":3.3876,"min_ms":29.4418,"max_ms":47.8633},{"document":"large","tool":"arity","mean_ms":791.3714,"ratio":1.0,"ratio_label":"baseline","stddev_ms":18.5129,"min_ms":773.7293,"max_ms":810.6469},{"document":"large","tool":"air","mean_ms":449.5672,"ratio":0.5680862361212448,"ratio_label":"1.8x faster","stddev_ms":80.7891,"min_ms":387.3011,"max_ms":588.7086}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per synthetic corpus tier. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>small (123094 bytes, 8498 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>26.3464</td><td>20.5507</td><td>36.0864</td><td>baseline</td></tr>
<tr><td>air</td><td>35.9680</td><td>29.4418</td><td>47.8633</td><td>1.4x slower</td></tr>
</tbody>
</table>
<h5>large (1477128 bytes, 101976 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>791.3714</td><td>773.7293</td><td>810.6469</td><td>baseline</td></tr>
<tr><td>air</td><td>449.5672</td><td>387.3011</td><td>588.7086</td><td>1.8x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":39.4097,"ratio":1.0,"ratio_label":"baseline","stddev_ms":4.0046,"min_ms":32.7052,"max_ms":49.4364},{"document":"tidyr","tool":"air","mean_ms":56.7434,"ratio":1.4398333405227648,"ratio_label":"1.4x slower","stddev_ms":5.0946,"min_ms":48.2065,"max_ms":70.0263}]</script>
<figcaption>Formatting speed on a real R package (the tidyr source tree) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>39.4097</td><td>32.7052</td><td>49.4364</td><td>baseline</td></tr>
<tr><td>air</td><td>56.7434</td><td>48.2065</td><td>70.0263</td><td>1.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"small","tool":"arity","mean_ms":37.6006,"ratio":1.0,"ratio_label":"baseline","stddev_ms":3.61,"min_ms":30.2579,"max_ms":44.9339},{"document":"small","tool":"jarl","mean_ms":44.5027,"ratio":1.1835635601559549,"ratio_label":"1.2x slower","stddev_ms":4.9298,"min_ms":32.822,"max_ms":55.5076},{"document":"large","tool":"arity","mean_ms":385.7874,"ratio":1.0,"ratio_label":"baseline","stddev_ms":43.3018,"min_ms":307.3075,"max_ms":435.3728},{"document":"large","tool":"jarl","mean_ms":673.2606,"ratio":1.7451596397394005,"ratio_label":"1.7x slower","stddev_ms":75.4781,"min_ms":628.6883,"max_ms":760.4076}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per synthetic corpus tier, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>small (123094 bytes, 8498 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>37.6006</td><td>30.2579</td><td>44.9339</td><td>baseline</td></tr>
<tr><td>jarl</td><td>44.5027</td><td>32.8220</td><td>55.5076</td><td>1.2x slower</td></tr>
</tbody>
</table>
<h5>large (1477128 bytes, 101976 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>385.7874</td><td>307.3075</td><td>435.3728</td><td>baseline</td></tr>
<tr><td>jarl</td><td>673.2606</td><td>628.6883</td><td>760.4076</td><td>1.7x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":34.4145,"ratio":1.0,"ratio_label":"baseline","stddev_ms":3.5912,"min_ms":28.2005,"max_ms":43.7791},{"document":"tidyr","tool":"jarl","mean_ms":23.1374,"ratio":0.6723154484301676,"ratio_label":"1.5x faster","stddev_ms":2.7275,"min_ms":17.6572,"max_ms":30.6717}]</script>
<figcaption>Linting speed on a real R package (the tidyr source tree) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>34.4145</td><td>28.2005</td><td>43.7791</td><td>baseline</td></tr>
<tr><td>jarl</td><td>23.1374</td><td>17.6572</td><td>30.6717</td><td>1.5x faster</td></tr>
</tbody>
</table>
</details>
</div>

