### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":4.2708,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.7985,"min_ms":3.2407,"max_ms":13.3517},{"document":"MASS/polr.R","tool":"air","mean_ms":4.7918,"ratio":1.1219911960288471,"ratio_label":"1.1x slower","stddev_ms":0.352,"min_ms":4.2234,"max_ms":5.9276},{"document":"MASS/polr.R","tool":"styler","mean_ms":2918.4963,"ratio":683.3605647653834,"ratio_label":"683.4x slower","stddev_ms":24.3648,"min_ms":2901.1808,"max_ms":2946.3576},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":3.6379,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.5564,"min_ms":2.7279,"max_ms":6.9879},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":3.1843,"ratio":0.8753126803925341,"ratio_label":"1.1x faster","stddev_ms":0.3783,"min_ms":2.6154,"max_ms":4.7064},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1748.9583,"ratio":480.7604112262569,"ratio_label":"480.8x slower","stddev_ms":66.4567,"min_ms":1683.6105,"max_ms":1816.4711},{"document":"small","tool":"arity","mean_ms":6.5863,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.1559,"min_ms":4.3103,"max_ms":18.1592},{"document":"small","tool":"air","mean_ms":13.9503,"ratio":2.1180784355404403,"ratio_label":"2.1x slower","stddev_ms":0.8177,"min_ms":12.5478,"max_ms":16.3397},{"document":"small","tool":"styler","mean_ms":14025.6846,"ratio":2129.5241030624175,"ratio_label":"2129.5x slower","stddev_ms":1138.7208,"min_ms":13138.7043,"max_ms":15309.7902},{"document":"large","tool":"arity","mean_ms":42.3533,"ratio":1.0,"ratio_label":"baseline","stddev_ms":6.5481,"min_ms":37.1178,"max_ms":68.5415},{"document":"large","tool":"air","mean_ms":133.9001,"ratio":3.1615033539299184,"ratio_label":"3.2x slower","stddev_ms":2.8556,"min_ms":129.9381,"max_ms":138.6219}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.2708</td><td>3.2407</td><td>13.3517</td><td>baseline</td></tr>
<tr><td>air</td><td>4.7918</td><td>4.2234</td><td>5.9276</td><td>1.1x slower</td></tr>
<tr><td>styler</td><td>2918.4963</td><td>2901.1808</td><td>2946.3576</td><td>683.4x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>3.6379</td><td>2.7279</td><td>6.9879</td><td>baseline</td></tr>
<tr><td>air</td><td>3.1843</td><td>2.6154</td><td>4.7064</td><td>1.1x faster</td></tr>
<tr><td>styler</td><td>1748.9583</td><td>1683.6105</td><td>1816.4711</td><td>480.8x slower</td></tr>
</tbody>
</table>
<h5>small (148330 bytes, 10170 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>6.5863</td><td>4.3103</td><td>18.1592</td><td>baseline</td></tr>
<tr><td>air</td><td>13.9503</td><td>12.5478</td><td>16.3397</td><td>2.1x slower</td></tr>
<tr><td>styler</td><td>14025.6846</td><td>13138.7043</td><td>15309.7902</td><td>2129.5x slower</td></tr>
</tbody>
</table>
<h5>large (1779960 bytes, 122040 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>42.3533</td><td>37.1178</td><td>68.5415</td><td>baseline</td></tr>
<tr><td>air</td><td>133.9001</td><td>129.9381</td><td>138.6219</td><td>3.2x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":20.686,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.6153,"min_ms":18.9913,"max_ms":22.0084},{"document":"tidyr","tool":"air","mean_ms":17.9184,"ratio":0.8662090302620129,"ratio_label":"1.2x faster","stddev_ms":0.6536,"min_ms":16.6688,"max_ms":19.5397},{"document":"MASS","tool":"arity","mean_ms":29.9188,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.154,"min_ms":28.0552,"max_ms":32.9356},{"document":"MASS","tool":"air","mean_ms":32.0778,"ratio":1.0721619851063546,"ratio_label":"1.1x slower","stddev_ms":0.915,"min_ms":30.4016,"max_ms":35.477}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>20.6860</td><td>18.9913</td><td>22.0084</td><td>baseline</td></tr>
<tr><td>air</td><td>17.9184</td><td>16.6688</td><td>19.5397</td><td>1.2x faster</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>29.9188</td><td>28.0552</td><td>32.9356</td><td>baseline</td></tr>
<tr><td>air</td><td>32.0778</td><td>30.4016</td><td>35.4770</td><td>1.1x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":62.7485,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.8875,"min_ms":58.7037,"max_ms":71.0253},{"document":"MASS/polr.R","tool":"jarl","mean_ms":15.2939,"ratio":0.24373331633425502,"ratio_label":"4.1x faster","stddev_ms":0.9333,"min_ms":13.8285,"max_ms":19.9717},{"document":"MASS/polr.R","tool":"lintr","mean_ms":836.7808,"ratio":13.335470967433485,"ratio_label":"13.3x slower","stddev_ms":40.9953,"min_ms":813.0816,"max_ms":884.118},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":46.2487,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.1534,"min_ms":43.4309,"max_ms":51.872},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":12.5729,"ratio":0.2718541277917001,"ratio_label":"3.7x faster","stddev_ms":0.7347,"min_ms":11.1225,"max_ms":14.9984},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":754.4522,"ratio":16.312938525839645,"ratio_label":"16.3x slower","stddev_ms":2.7409,"min_ms":751.9923,"max_ms":757.4068},{"document":"small","tool":"arity","mean_ms":115.8899,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.3816,"min_ms":112.6254,"max_ms":122.4819},{"document":"small","tool":"jarl","mean_ms":26.5493,"ratio":0.2290907145488951,"ratio_label":"4.4x faster","stddev_ms":0.8713,"min_ms":25.3481,"max_ms":29.1048},{"document":"small","tool":"lintr","mean_ms":7338.657,"ratio":63.32438806142727,"ratio_label":"63.3x slower","stddev_ms":44.3214,"min_ms":7305.3569,"max_ms":7388.9629},{"document":"large","tool":"arity","mean_ms":11477.3402,"ratio":1.0,"ratio_label":"baseline","stddev_ms":203.0556,"min_ms":11357.9114,"max_ms":11711.7949},{"document":"large","tool":"jarl","mean_ms":802.2347,"ratio":0.06989726591880582,"ratio_label":"14.3x faster","stddev_ms":2.6697,"min_ms":799.4166,"max_ms":804.726}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>62.7485</td><td>58.7037</td><td>71.0253</td><td>baseline</td></tr>
<tr><td>jarl</td><td>15.2939</td><td>13.8285</td><td>19.9717</td><td>4.1x faster</td></tr>
<tr><td>lintr</td><td>836.7808</td><td>813.0816</td><td>884.1180</td><td>13.3x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>46.2487</td><td>43.4309</td><td>51.8720</td><td>baseline</td></tr>
<tr><td>jarl</td><td>12.5729</td><td>11.1225</td><td>14.9984</td><td>3.7x faster</td></tr>
<tr><td>lintr</td><td>754.4522</td><td>751.9923</td><td>757.4068</td><td>16.3x slower</td></tr>
</tbody>
</table>
<h5>small (148330 bytes, 10170 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>115.8899</td><td>112.6254</td><td>122.4819</td><td>baseline</td></tr>
<tr><td>jarl</td><td>26.5493</td><td>25.3481</td><td>29.1048</td><td>4.4x faster</td></tr>
<tr><td>lintr</td><td>7338.6570</td><td>7305.3569</td><td>7388.9629</td><td>63.3x slower</td></tr>
</tbody>
</table>
<h5>large (1779960 bytes, 122040 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>11477.3402</td><td>11357.9114</td><td>11711.7949</td><td>baseline</td></tr>
<tr><td>jarl</td><td>802.2347</td><td>799.4166</td><td>804.7260</td><td>14.3x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":49.1694,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.5958,"min_ms":45.1694,"max_ms":51.9856},{"document":"tidyr","tool":"jarl","mean_ms":14.0102,"ratio":0.28493737975244765,"ratio_label":"3.5x faster","stddev_ms":0.6323,"min_ms":12.4196,"max_ms":15.6445},{"document":"tidyr","tool":"lintr","mean_ms":7178.4927,"ratio":145.9951250167787,"ratio_label":"146.0x slower","stddev_ms":134.3254,"min_ms":7027.6252,"max_ms":7285.1122},{"document":"MASS","tool":"arity","mean_ms":65.9216,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.2593,"min_ms":61.4211,"max_ms":71.3542},{"document":"MASS","tool":"jarl","mean_ms":18.9814,"ratio":0.28793900633479774,"ratio_label":"3.5x faster","stddev_ms":0.9256,"min_ms":17.1902,"max_ms":21.4288},{"document":"MASS","tool":"lintr","mean_ms":6837.9611,"ratio":103.72868832067184,"ratio_label":"103.7x slower","stddev_ms":51.4358,"min_ms":6790.6173,"max_ms":6892.6904}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>49.1694</td><td>45.1694</td><td>51.9856</td><td>baseline</td></tr>
<tr><td>jarl</td><td>14.0102</td><td>12.4196</td><td>15.6445</td><td>3.5x faster</td></tr>
<tr><td>lintr</td><td>7178.4927</td><td>7027.6252</td><td>7285.1122</td><td>146.0x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>65.9216</td><td>61.4211</td><td>71.3542</td><td>baseline</td></tr>
<tr><td>jarl</td><td>18.9814</td><td>17.1902</td><td>21.4288</td><td>3.5x faster</td></tr>
<tr><td>lintr</td><td>6837.9611</td><td>6790.6173</td><td>6892.6904</td><td>103.7x slower</td></tr>
</tbody>
</table>
</details>
</div>

