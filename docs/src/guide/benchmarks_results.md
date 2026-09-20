### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":4.8415,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.6281,"min_ms":3.6281,"max_ms":9.1041},{"document":"MASS/polr.R","tool":"air","mean_ms":8.2043,"ratio":1.6945781266136528,"ratio_label":"1.7x slower","stddev_ms":0.5961,"min_ms":7.0727,"max_ms":12.0133},{"document":"MASS/polr.R","tool":"styler","mean_ms":3164.6802,"ratio":653.6569658163792,"ratio_label":"653.7x slower","stddev_ms":122.3425,"min_ms":3060.0818,"max_ms":3299.2111},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":4.4597,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.6419,"min_ms":3.2025,"max_ms":10.6042},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":5.6126,"ratio":1.2585151467587505,"ratio_label":"1.3x slower","stddev_ms":0.6007,"min_ms":4.5239,"max_ms":9.8578},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1737.511,"ratio":389.6026638563132,"ratio_label":"389.6x slower","stddev_ms":29.3376,"min_ms":1718.2032,"max_ms":1771.271},{"document":"small","tool":"arity","mean_ms":6.2978,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.986,"min_ms":4.8161,"max_ms":14.9822},{"document":"small","tool":"air","mean_ms":29.8889,"ratio":4.7459271491632,"ratio_label":"4.7x slower","stddev_ms":1.6604,"min_ms":26.5463,"max_ms":34.6716},{"document":"small","tool":"styler","mean_ms":13353.8912,"ratio":2120.405728984725,"ratio_label":"2120.4x slower","stddev_ms":307.7941,"min_ms":13063.7314,"max_ms":13676.7144},{"document":"large","tool":"arity","mean_ms":45.0165,"ratio":1.0,"ratio_label":"baseline","stddev_ms":14.3207,"min_ms":36.522,"max_ms":107.3092},{"document":"large","tool":"air","mean_ms":332.7182,"ratio":7.391027734275211,"ratio_label":"7.4x slower","stddev_ms":3.7626,"min_ms":325.1847,"max_ms":337.4778}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.8415</td><td>3.6281</td><td>9.1041</td><td>baseline</td></tr>
<tr><td>air</td><td>8.2043</td><td>7.0727</td><td>12.0133</td><td>1.7x slower</td></tr>
<tr><td>styler</td><td>3164.6802</td><td>3060.0818</td><td>3299.2111</td><td>653.7x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.4597</td><td>3.2025</td><td>10.6042</td><td>baseline</td></tr>
<tr><td>air</td><td>5.6126</td><td>4.5239</td><td>9.8578</td><td>1.3x slower</td></tr>
<tr><td>styler</td><td>1737.5110</td><td>1718.2032</td><td>1771.2710</td><td>389.6x slower</td></tr>
</tbody>
</table>
<h5>small (147122 bytes, 9896 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>6.2978</td><td>4.8161</td><td>14.9822</td><td>baseline</td></tr>
<tr><td>air</td><td>29.8889</td><td>26.5463</td><td>34.6716</td><td>4.7x slower</td></tr>
<tr><td>styler</td><td>13353.8912</td><td>13063.7314</td><td>13676.7144</td><td>2120.4x slower</td></tr>
</tbody>
</table>
<h5>large (1765464 bytes, 118752 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>45.0165</td><td>36.5220</td><td>107.3092</td><td>baseline</td></tr>
<tr><td>air</td><td>332.7182</td><td>325.1847</td><td>337.4778</td><td>7.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":23.7209,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.1579,"min_ms":20.9512,"max_ms":37.4993},{"document":"tidyr","tool":"air","mean_ms":36.1422,"ratio":1.5236437066047241,"ratio_label":"1.5x slower","stddev_ms":1.11,"min_ms":33.7529,"max_ms":39.3543},{"document":"MASS","tool":"arity","mean_ms":33.3608,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.71,"min_ms":31.1454,"max_ms":42.0423},{"document":"MASS","tool":"air","mean_ms":63.3019,"ratio":1.8974934653845235,"ratio_label":"1.9x slower","stddev_ms":1.8568,"min_ms":59.795,"max_ms":67.5271}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>23.7209</td><td>20.9512</td><td>37.4993</td><td>baseline</td></tr>
<tr><td>air</td><td>36.1422</td><td>33.7529</td><td>39.3543</td><td>1.5x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>33.3608</td><td>31.1454</td><td>42.0423</td><td>baseline</td></tr>
<tr><td>air</td><td>63.3019</td><td>59.7950</td><td>67.5271</td><td>1.9x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":33.1615,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.8477,"min_ms":29.7368,"max_ms":39.1127},{"document":"MASS/polr.R","tool":"jarl","mean_ms":12.9628,"ratio":0.3908990847820515,"ratio_label":"2.6x faster","stddev_ms":0.5789,"min_ms":11.9096,"max_ms":16.1438},{"document":"MASS/polr.R","tool":"lintr","mean_ms":1065.3256,"ratio":32.1253743045399,"ratio_label":"32.1x slower","stddev_ms":5.5348,"min_ms":1058.9672,"max_ms":1069.0634},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":30.8709,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.7786,"min_ms":27.8587,"max_ms":41.4557},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":10.6234,"ratio":0.34412343015590735,"ratio_label":"2.9x faster","stddev_ms":0.6517,"min_ms":9.5503,"max_ms":14.3922},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":902.9177,"ratio":29.248181944808866,"ratio_label":"29.2x slower","stddev_ms":10.5485,"min_ms":893.4831,"max_ms":914.3067},{"document":"small","tool":"arity","mean_ms":30.1509,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.0012,"min_ms":28.3004,"max_ms":33.713},{"document":"small","tool":"jarl","mean_ms":31.1111,"ratio":1.0318464788779107,"ratio_label":"1.0x slower","stddev_ms":1.271,"min_ms":28.95,"max_ms":34.17},{"document":"small","tool":"lintr","mean_ms":13989.4429,"ratio":463.9809392091115,"ratio_label":"464.0x slower","stddev_ms":1890.3543,"min_ms":11809.6553,"max_ms":15178.5032},{"document":"large","tool":"arity","mean_ms":728.6282,"ratio":1.0,"ratio_label":"baseline","stddev_ms":252.8363,"min_ms":572.0615,"max_ms":1020.3157},{"document":"large","tool":"jarl","mean_ms":492.3987,"ratio":0.6757886944260462,"ratio_label":"1.5x faster","stddev_ms":28.2576,"min_ms":471.999,"max_ms":547.0819}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>33.1615</td><td>29.7368</td><td>39.1127</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.9628</td><td>11.9096</td><td>16.1438</td><td>2.6x faster</td></tr>
<tr><td>lintr</td><td>1065.3256</td><td>1058.9672</td><td>1069.0634</td><td>32.1x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>30.8709</td><td>27.8587</td><td>41.4557</td><td>baseline</td></tr>
<tr><td>jarl</td><td>10.6234</td><td>9.5503</td><td>14.3922</td><td>2.9x faster</td></tr>
<tr><td>lintr</td><td>902.9177</td><td>893.4831</td><td>914.3067</td><td>29.2x slower</td></tr>
</tbody>
</table>
<h5>small (147122 bytes, 9896 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>30.1509</td><td>28.3004</td><td>33.7130</td><td>baseline</td></tr>
<tr><td>jarl</td><td>31.1111</td><td>28.9500</td><td>34.1700</td><td>1.0x slower</td></tr>
<tr><td>lintr</td><td>13989.4429</td><td>11809.6553</td><td>15178.5032</td><td>464.0x slower</td></tr>
</tbody>
</table>
<h5>large (1765464 bytes, 118752 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>728.6282</td><td>572.0615</td><td>1020.3157</td><td>baseline</td></tr>
<tr><td>jarl</td><td>492.3987</td><td>471.9990</td><td>547.0819</td><td>1.5x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":32.1305,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.6109,"min_ms":28.5647,"max_ms":37.8632},{"document":"tidyr","tool":"jarl","mean_ms":13.8226,"ratio":0.4302018331491885,"ratio_label":"2.3x faster","stddev_ms":0.9254,"min_ms":12.2326,"max_ms":19.1293},{"document":"tidyr","tool":"lintr","mean_ms":9238.4568,"ratio":287.529195001634,"ratio_label":"287.5x slower","stddev_ms":61.0105,"min_ms":9186.3826,"max_ms":9305.5851},{"document":"MASS","tool":"arity","mean_ms":35.8673,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.9851,"min_ms":30.994,"max_ms":40.1854},{"document":"MASS","tool":"jarl","mean_ms":20.561,"ratio":0.5732519593055513,"ratio_label":"1.7x faster","stddev_ms":2.0127,"min_ms":16.7284,"max_ms":28.7741},{"document":"MASS","tool":"lintr","mean_ms":9519.1274,"ratio":265.3984938927658,"ratio_label":"265.4x slower","stddev_ms":80.0788,"min_ms":9438.2252,"max_ms":9598.3566}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>32.1305</td><td>28.5647</td><td>37.8632</td><td>baseline</td></tr>
<tr><td>jarl</td><td>13.8226</td><td>12.2326</td><td>19.1293</td><td>2.3x faster</td></tr>
<tr><td>lintr</td><td>9238.4568</td><td>9186.3826</td><td>9305.5851</td><td>287.5x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>35.8673</td><td>30.9940</td><td>40.1854</td><td>baseline</td></tr>
<tr><td>jarl</td><td>20.5610</td><td>16.7284</td><td>28.7741</td><td>1.7x faster</td></tr>
<tr><td>lintr</td><td>9519.1274</td><td>9438.2252</td><td>9598.3566</td><td>265.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

