### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":4.2482,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.441,"min_ms":3.3172,"max_ms":6.9881},{"document":"MASS/polr.R","tool":"air","mean_ms":4.9524,"ratio":1.1657643237135729,"ratio_label":"1.2x slower","stddev_ms":0.5032,"min_ms":4.1557,"max_ms":9.2375},{"document":"MASS/polr.R","tool":"styler","mean_ms":3029.3444,"ratio":713.0889317828728,"ratio_label":"713.1x slower","stddev_ms":18.953,"min_ms":3016.0064,"max_ms":3051.0396},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":4.2974,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.7705,"min_ms":2.8382,"max_ms":8.7005},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":3.5158,"ratio":0.818122585749523,"ratio_label":"1.2x faster","stddev_ms":0.9307,"min_ms":2.6664,"max_ms":7.6818},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1757.9622,"ratio":409.07576674268165,"ratio_label":"409.1x slower","stddev_ms":10.3743,"min_ms":1745.9831,"max_ms":1764.0001},{"document":"small","tool":"arity","mean_ms":5.6885,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.5803,"min_ms":4.6001,"max_ms":8.3916},{"document":"small","tool":"air","mean_ms":14.4204,"ratio":2.5350092291465236,"ratio_label":"2.5x slower","stddev_ms":0.9259,"min_ms":12.8599,"max_ms":18.0106},{"document":"small","tool":"styler","mean_ms":13263.0925,"ratio":2331.56236266151,"ratio_label":"2331.6x slower","stddev_ms":200.8928,"min_ms":13047.5309,"max_ms":13445.088},{"document":"large","tool":"arity","mean_ms":40.2795,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.9219,"min_ms":36.9963,"max_ms":47.9367},{"document":"large","tool":"air","mean_ms":137.5681,"ratio":3.415337826934296,"ratio_label":"3.4x slower","stddev_ms":3.6819,"min_ms":132.2173,"max_ms":148.4001}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.2482</td><td>3.3172</td><td>6.9881</td><td>baseline</td></tr>
<tr><td>air</td><td>4.9524</td><td>4.1557</td><td>9.2375</td><td>1.2x slower</td></tr>
<tr><td>styler</td><td>3029.3444</td><td>3016.0064</td><td>3051.0396</td><td>713.1x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.2974</td><td>2.8382</td><td>8.7005</td><td>baseline</td></tr>
<tr><td>air</td><td>3.5158</td><td>2.6664</td><td>7.6818</td><td>1.2x faster</td></tr>
<tr><td>styler</td><td>1757.9622</td><td>1745.9831</td><td>1764.0001</td><td>409.1x slower</td></tr>
</tbody>
</table>
<h5>small (148330 bytes, 10170 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>5.6885</td><td>4.6001</td><td>8.3916</td><td>baseline</td></tr>
<tr><td>air</td><td>14.4204</td><td>12.8599</td><td>18.0106</td><td>2.5x slower</td></tr>
<tr><td>styler</td><td>13263.0925</td><td>13047.5309</td><td>13445.0880</td><td>2331.6x slower</td></tr>
</tbody>
</table>
<h5>large (1779960 bytes, 122040 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>40.2795</td><td>36.9963</td><td>47.9367</td><td>baseline</td></tr>
<tr><td>air</td><td>137.5681</td><td>132.2173</td><td>148.4001</td><td>3.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":20.9329,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.64,"min_ms":19.3785,"max_ms":24.0346},{"document":"tidyr","tool":"air","mean_ms":18.0498,"ratio":0.8622694418833512,"ratio_label":"1.2x faster","stddev_ms":0.6336,"min_ms":17.0653,"max_ms":20.0314},{"document":"MASS","tool":"arity","mean_ms":31.3859,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.6941,"min_ms":29.4117,"max_ms":32.7962},{"document":"MASS","tool":"air","mean_ms":32.2078,"ratio":1.0261869183295684,"ratio_label":"1.0x slower","stddev_ms":0.7499,"min_ms":30.443,"max_ms":34.1248}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>20.9329</td><td>19.3785</td><td>24.0346</td><td>baseline</td></tr>
<tr><td>air</td><td>18.0498</td><td>17.0653</td><td>20.0314</td><td>1.2x faster</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>31.3859</td><td>29.4117</td><td>32.7962</td><td>baseline</td></tr>
<tr><td>air</td><td>32.2078</td><td>30.4430</td><td>34.1248</td><td>1.0x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":1478.2241,"ratio":1.0,"ratio_label":"baseline","stddev_ms":4.2591,"min_ms":1475.6609,"max_ms":1483.1406},{"document":"MASS/polr.R","tool":"jarl","mean_ms":12.6975,"ratio":0.008589698950247124,"ratio_label":"116.4x faster","stddev_ms":0.6044,"min_ms":11.8166,"max_ms":14.793},{"document":"MASS/polr.R","tool":"lintr","mean_ms":1063.4596,"ratio":0.7194170356172653,"ratio_label":"1.4x faster","stddev_ms":62.6372,"min_ms":1022.7495,"max_ms":1135.5876},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":195.5751,"ratio":1.0,"ratio_label":"baseline","stddev_ms":15.7732,"min_ms":184.6347,"max_ms":242.1064},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":10.9518,"ratio":0.055997926116361445,"ratio_label":"17.9x faster","stddev_ms":0.8413,"min_ms":9.4614,"max_ms":14.325},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":901.233,"ratio":4.608117291004836,"ratio_label":"4.6x slower","stddev_ms":7.4306,"min_ms":895.3008,"max_ms":909.5676},{"document":"small","tool":"arity","mean_ms":227.3363,"ratio":1.0,"ratio_label":"baseline","stddev_ms":13.7653,"min_ms":204.9546,"max_ms":252.2441},{"document":"small","tool":"jarl","mean_ms":78.3887,"ratio":0.3448138286758428,"ratio_label":"2.9x faster","stddev_ms":12.0517,"min_ms":58.9732,"max_ms":100.2281},{"document":"small","tool":"lintr","mean_ms":12306.2324,"ratio":54.13228067844863,"ratio_label":"54.1x slower","stddev_ms":583.402,"min_ms":11852.6386,"max_ms":12964.3614},{"document":"large","tool":"arity","mean_ms":11339.7209,"ratio":1.0,"ratio_label":"baseline","stddev_ms":105.1464,"min_ms":11228.8875,"max_ms":11438.0647},{"document":"large","tool":"jarl","mean_ms":452.5446,"ratio":0.039907913430215027,"ratio_label":"25.1x faster","stddev_ms":2.6254,"min_ms":449.455,"max_ms":456.3735}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>1478.2241</td><td>1475.6609</td><td>1483.1406</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.6975</td><td>11.8166</td><td>14.7930</td><td>116.4x faster</td></tr>
<tr><td>lintr</td><td>1063.4596</td><td>1022.7495</td><td>1135.5876</td><td>1.4x faster</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>195.5751</td><td>184.6347</td><td>242.1064</td><td>baseline</td></tr>
<tr><td>jarl</td><td>10.9518</td><td>9.4614</td><td>14.3250</td><td>17.9x faster</td></tr>
<tr><td>lintr</td><td>901.2330</td><td>895.3008</td><td>909.5676</td><td>4.6x slower</td></tr>
</tbody>
</table>
<h5>small (148330 bytes, 10170 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>227.3363</td><td>204.9546</td><td>252.2441</td><td>baseline</td></tr>
<tr><td>jarl</td><td>78.3887</td><td>58.9732</td><td>100.2281</td><td>2.9x faster</td></tr>
<tr><td>lintr</td><td>12306.2324</td><td>11852.6386</td><td>12964.3614</td><td>54.1x slower</td></tr>
</tbody>
</table>
<h5>large (1779960 bytes, 122040 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>11339.7209</td><td>11228.8875</td><td>11438.0647</td><td>baseline</td></tr>
<tr><td>jarl</td><td>452.5446</td><td>449.4550</td><td>456.3735</td><td>25.1x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":167.3592,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.9954,"min_ms":163.5778,"max_ms":175.0285},{"document":"tidyr","tool":"jarl","mean_ms":13.6388,"ratio":0.08149417540236809,"ratio_label":"12.3x faster","stddev_ms":0.9576,"min_ms":11.8054,"max_ms":17.6327},{"document":"tidyr","tool":"lintr","mean_ms":8621.3587,"ratio":51.514100808321274,"ratio_label":"51.5x slower","stddev_ms":102.4746,"min_ms":8504.0474,"max_ms":8693.4148},{"document":"MASS","tool":"arity","mean_ms":1352.8051,"ratio":1.0,"ratio_label":"baseline","stddev_ms":15.6886,"min_ms":1334.7105,"max_ms":1362.6076},{"document":"MASS","tool":"jarl","mean_ms":17.8636,"ratio":0.01320485855649125,"ratio_label":"75.7x faster","stddev_ms":1.0976,"min_ms":15.8221,"max_ms":21.1379},{"document":"MASS","tool":"lintr","mean_ms":8683.6601,"ratio":6.41900307738343,"ratio_label":"6.4x slower","stddev_ms":53.1637,"min_ms":8648.9795,"max_ms":8744.8674}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>167.3592</td><td>163.5778</td><td>175.0285</td><td>baseline</td></tr>
<tr><td>jarl</td><td>13.6388</td><td>11.8054</td><td>17.6327</td><td>12.3x faster</td></tr>
<tr><td>lintr</td><td>8621.3587</td><td>8504.0474</td><td>8693.4148</td><td>51.5x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>1352.8051</td><td>1334.7105</td><td>1362.6076</td><td>baseline</td></tr>
<tr><td>jarl</td><td>17.8636</td><td>15.8221</td><td>21.1379</td><td>75.7x faster</td></tr>
<tr><td>lintr</td><td>8683.6601</td><td>8648.9795</td><td>8744.8674</td><td>6.4x slower</td></tr>
</tbody>
</table>
</details>
</div>

