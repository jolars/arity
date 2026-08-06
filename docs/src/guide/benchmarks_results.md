### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":5.8455,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.4767,"min_ms":4.9571,"max_ms":7.9215},{"document":"MASS/polr.R","tool":"air","mean_ms":7.9158,"ratio":1.354169874262253,"ratio_label":"1.4x slower","stddev_ms":0.4918,"min_ms":7.0364,"max_ms":9.4193},{"document":"MASS/polr.R","tool":"styler","mean_ms":3005.2278,"ratio":514.109622786759,"ratio_label":"514.1x slower","stddev_ms":16.7858,"min_ms":2991.9473,"max_ms":3024.0944},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":4.66,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.4174,"min_ms":3.9505,"max_ms":6.9153},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":5.2126,"ratio":1.1185836909871245,"ratio_label":"1.1x slower","stddev_ms":0.4361,"min_ms":4.51,"max_ms":8.4923},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1657.7481,"ratio":355.7399356223176,"ratio_label":"355.7x slower","stddev_ms":20.3054,"min_ms":1637.9878,"max_ms":1678.5579},{"document":"small","tool":"arity","mean_ms":18.8899,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.2909,"min_ms":16.4381,"max_ms":24.1338},{"document":"small","tool":"air","mean_ms":24.7004,"ratio":1.3075982403294881,"ratio_label":"1.3x slower","stddev_ms":1.2469,"min_ms":22.6597,"max_ms":28.3625},{"document":"small","tool":"styler","mean_ms":11257.2645,"ratio":595.9409261033674,"ratio_label":"595.9x slower","stddev_ms":27.5045,"min_ms":11235.0162,"max_ms":11288.0166},{"document":"large","tool":"arity","mean_ms":472.637,"ratio":1.0,"ratio_label":"baseline","stddev_ms":11.2247,"min_ms":460.7096,"max_ms":493.4121},{"document":"large","tool":"air","mean_ms":311.0487,"ratio":0.6581133089453428,"ratio_label":"1.5x faster","stddev_ms":9.7044,"min_ms":297.7228,"max_ms":329.7785}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>5.8455</td><td>4.9571</td><td>7.9215</td><td>baseline</td></tr>
<tr><td>air</td><td>7.9158</td><td>7.0364</td><td>9.4193</td><td>1.4x slower</td></tr>
<tr><td>styler</td><td>3005.2278</td><td>2991.9473</td><td>3024.0944</td><td>514.1x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.6600</td><td>3.9505</td><td>6.9153</td><td>baseline</td></tr>
<tr><td>air</td><td>5.2126</td><td>4.5100</td><td>8.4923</td><td>1.1x slower</td></tr>
<tr><td>styler</td><td>1657.7481</td><td>1637.9878</td><td>1678.5579</td><td>355.7x slower</td></tr>
</tbody>
</table>
<h5>small (128902 bytes, 8892 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>18.8899</td><td>16.4381</td><td>24.1338</td><td>baseline</td></tr>
<tr><td>air</td><td>24.7004</td><td>22.6597</td><td>28.3625</td><td>1.3x slower</td></tr>
<tr><td>styler</td><td>11257.2645</td><td>11235.0162</td><td>11288.0166</td><td>595.9x slower</td></tr>
</tbody>
</table>
<h5>large (1546824 bytes, 106704 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>472.6370</td><td>460.7096</td><td>493.4121</td><td>baseline</td></tr>
<tr><td>air</td><td>311.0487</td><td>297.7228</td><td>329.7785</td><td>1.5x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":25.4796,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.9609,"min_ms":23.6112,"max_ms":28.0976},{"document":"tidyr","tool":"air","mean_ms":35.4748,"ratio":1.3922824534137113,"ratio_label":"1.4x slower","stddev_ms":0.9484,"min_ms":33.9106,"max_ms":38.0428},{"document":"MASS","tool":"arity","mean_ms":50.8426,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.2419,"min_ms":48.8145,"max_ms":53.4183},{"document":"MASS","tool":"air","mean_ms":61.4861,"ratio":1.2093421658215748,"ratio_label":"1.2x slower","stddev_ms":1.3215,"min_ms":59.4653,"max_ms":65.0481}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>25.4796</td><td>23.6112</td><td>28.0976</td><td>baseline</td></tr>
<tr><td>air</td><td>35.4748</td><td>33.9106</td><td>38.0428</td><td>1.4x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>50.8426</td><td>48.8145</td><td>53.4183</td><td>baseline</td></tr>
<tr><td>air</td><td>61.4861</td><td>59.4653</td><td>65.0481</td><td>1.2x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":26.2369,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.5585,"min_ms":23.8034,"max_ms":31.0467},{"document":"MASS/polr.R","tool":"jarl","mean_ms":12.8946,"ratio":0.4914681231395478,"ratio_label":"2.0x faster","stddev_ms":0.5434,"min_ms":11.9362,"max_ms":15.0183},{"document":"MASS/polr.R","tool":"lintr","mean_ms":1106.3045,"ratio":42.16597616334247,"ratio_label":"42.2x slower","stddev_ms":17.2817,"min_ms":1088.3735,"max_ms":1122.8541},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":20.9963,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.9218,"min_ms":18.8055,"max_ms":23.3214},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":10.9198,"ratio":0.5200821097050432,"ratio_label":"1.9x faster","stddev_ms":0.578,"min_ms":9.4974,"max_ms":12.8576},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":995.9262,"ratio":47.433414458737964,"ratio_label":"47.4x slower","stddev_ms":15.6979,"min_ms":986.5383,"max_ms":1014.0487},{"document":"small","tool":"arity","mean_ms":32.5569,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.4612,"min_ms":29.8406,"max_ms":36.1271},{"document":"small","tool":"jarl","mean_ms":28.3228,"ratio":0.8699476915799723,"ratio_label":"1.1x faster","stddev_ms":1.2537,"min_ms":26.0793,"max_ms":33.789},{"document":"small","tool":"lintr","mean_ms":9949.3436,"ratio":305.5986165759025,"ratio_label":"305.6x slower","stddev_ms":94.5368,"min_ms":9893.0505,"max_ms":10058.4872},{"document":"large","tool":"arity","mean_ms":343.8745,"ratio":1.0,"ratio_label":"baseline","stddev_ms":8.7147,"min_ms":335.9537,"max_ms":363.002},{"document":"large","tool":"jarl","mean_ms":393.3544,"ratio":1.1438894131434578,"ratio_label":"1.1x slower","stddev_ms":8.1381,"min_ms":385.9586,"max_ms":410.6601}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>26.2369</td><td>23.8034</td><td>31.0467</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.8946</td><td>11.9362</td><td>15.0183</td><td>2.0x faster</td></tr>
<tr><td>lintr</td><td>1106.3045</td><td>1088.3735</td><td>1122.8541</td><td>42.2x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>20.9963</td><td>18.8055</td><td>23.3214</td><td>baseline</td></tr>
<tr><td>jarl</td><td>10.9198</td><td>9.4974</td><td>12.8576</td><td>1.9x faster</td></tr>
<tr><td>lintr</td><td>995.9262</td><td>986.5383</td><td>1014.0487</td><td>47.4x slower</td></tr>
</tbody>
</table>
<h5>small (128902 bytes, 8892 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>32.5569</td><td>29.8406</td><td>36.1271</td><td>baseline</td></tr>
<tr><td>jarl</td><td>28.3228</td><td>26.0793</td><td>33.7890</td><td>1.1x faster</td></tr>
<tr><td>lintr</td><td>9949.3436</td><td>9893.0505</td><td>10058.4872</td><td>305.6x slower</td></tr>
</tbody>
</table>
<h5>large (1546824 bytes, 106704 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>343.8745</td><td>335.9537</td><td>363.0020</td><td>baseline</td></tr>
<tr><td>jarl</td><td>393.3544</td><td>385.9586</td><td>410.6601</td><td>1.1x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":23.4758,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.9418,"min_ms":21.8269,"max_ms":27.5558},{"document":"tidyr","tool":"jarl","mean_ms":13.0743,"ratio":0.5569267075030456,"ratio_label":"1.8x faster","stddev_ms":0.6655,"min_ms":11.91,"max_ms":15.6657},{"document":"tidyr","tool":"lintr","mean_ms":8417.3967,"ratio":358.5563303486995,"ratio_label":"358.6x slower","stddev_ms":36.636,"min_ms":8391.5407,"max_ms":8459.3213},{"document":"MASS","tool":"arity","mean_ms":27.6244,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.0801,"min_ms":25.7413,"max_ms":30.8305},{"document":"MASS","tool":"jarl","mean_ms":17.8624,"ratio":0.6466167590970302,"ratio_label":"1.5x faster","stddev_ms":0.9985,"min_ms":16.162,"max_ms":22.1038},{"document":"MASS","tool":"lintr","mean_ms":8694.5697,"ratio":314.7423907849582,"ratio_label":"314.7x slower","stddev_ms":38.7733,"min_ms":8652.4471,"max_ms":8728.77}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>23.4758</td><td>21.8269</td><td>27.5558</td><td>baseline</td></tr>
<tr><td>jarl</td><td>13.0743</td><td>11.9100</td><td>15.6657</td><td>1.8x faster</td></tr>
<tr><td>lintr</td><td>8417.3967</td><td>8391.5407</td><td>8459.3213</td><td>358.6x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>27.6244</td><td>25.7413</td><td>30.8305</td><td>baseline</td></tr>
<tr><td>jarl</td><td>17.8624</td><td>16.1620</td><td>22.1038</td><td>1.5x faster</td></tr>
<tr><td>lintr</td><td>8694.5697</td><td>8652.4471</td><td>8728.7700</td><td>314.7x slower</td></tr>
</tbody>
</table>
</details>
</div>

