### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"small","tool":"arity","mean_ms":28.9723,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.1666,"min_ms":24.1047,"max_ms":34.3484},{"document":"small","tool":"air","mean_ms":34.8371,"ratio":1.2024278362435843,"ratio_label":"1.2x slower","stddev_ms":2.9993,"min_ms":29.534,"max_ms":42.9128},{"document":"large","tool":"arity","mean_ms":804.4884,"ratio":1.0,"ratio_label":"baseline","stddev_ms":14.4069,"min_ms":794.7676,"max_ms":821.0402},{"document":"large","tool":"air","mean_ms":395.0042,"ratio":0.4910004917410867,"ratio_label":"2.0x faster","stddev_ms":16.1603,"min_ms":373.4905,"max_ms":414.9392}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per synthetic corpus tier. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>small (123094 bytes, 8498 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>28.9723</td><td>24.1047</td><td>34.3484</td><td>baseline</td></tr>
<tr><td>air</td><td>34.8371</td><td>29.5340</td><td>42.9128</td><td>1.2x slower</td></tr>
</tbody>
</table>
<h5>large (1477128 bytes, 101976 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>804.4884</td><td>794.7676</td><td>821.0402</td><td>baseline</td></tr>
<tr><td>air</td><td>395.0042</td><td>373.4905</td><td>414.9392</td><td>2.0x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":44.2919,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.5445,"min_ms":39.4409,"max_ms":52.2134},{"document":"tidyr","tool":"air","mean_ms":52.5817,"ratio":1.1871628898286142,"ratio_label":"1.2x slower","stddev_ms":2.6026,"min_ms":47.6156,"max_ms":58.4515}]</script>
<figcaption>Formatting speed on a real R package (the tidyr source tree) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>44.2919</td><td>39.4409</td><td>52.2134</td><td>baseline</td></tr>
<tr><td>air</td><td>52.5817</td><td>47.6156</td><td>58.4515</td><td>1.2x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"small","tool":"arity","mean_ms":54.4162,"ratio":1.0,"ratio_label":"baseline","stddev_ms":4.87,"min_ms":47.65,"max_ms":68.7513},{"document":"small","tool":"jarl","mean_ms":37.7893,"ratio":0.6944494470396683,"ratio_label":"1.4x faster","stddev_ms":3.0096,"min_ms":32.7067,"max_ms":47.5714},{"document":"large","tool":"arity","mean_ms":391.3159,"ratio":1.0,"ratio_label":"baseline","stddev_ms":25.2648,"min_ms":359.4516,"max_ms":423.6041},{"document":"large","tool":"jarl","mean_ms":520.3245,"ratio":1.3296789115903545,"ratio_label":"1.3x slower","stddev_ms":4.414,"min_ms":515.8793,"max_ms":527.6512}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per synthetic corpus tier, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>small (123094 bytes, 8498 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>54.4162</td><td>47.6500</td><td>68.7513</td><td>baseline</td></tr>
<tr><td>jarl</td><td>37.7893</td><td>32.7067</td><td>47.5714</td><td>1.4x faster</td></tr>
</tbody>
</table>
<h5>large (1477128 bytes, 101976 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>391.3159</td><td>359.4516</td><td>423.6041</td><td>baseline</td></tr>
<tr><td>jarl</td><td>520.3245</td><td>515.8793</td><td>527.6512</td><td>1.3x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":75.5673,"ratio":1.0,"ratio_label":"baseline","stddev_ms":4.7311,"min_ms":69.0003,"max_ms":91.4882},{"document":"tidyr","tool":"jarl","mean_ms":19.4321,"ratio":0.25714958718916775,"ratio_label":"3.9x faster","stddev_ms":1.9103,"min_ms":15.8706,"max_ms":31.6497}]</script>
<figcaption>Linting speed on a real R package (the tidyr source tree) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>75.5673</td><td>69.0003</td><td>91.4882</td><td>baseline</td></tr>
<tr><td>jarl</td><td>19.4321</td><td>15.8706</td><td>31.6497</td><td>3.9x faster</td></tr>
</tbody>
</table>
</details>
</div>

