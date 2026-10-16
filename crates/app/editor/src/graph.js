// Commit graph lanes. Commits arrive newest first in topological order; each
// lane holds the hash it waits for next. One row of the graph is drawn per
// commit: lines passing through, lines joining the commit from above, and
// lines leaving it towards its parents.

export const LANE_W = 11;
export const COLORS = 8;
const PAD = 7;

export function createGraph() {
  return { lanes: [], colors: [], next: 0, rows: [], width: 1 };
}

function free(lanes) {
  const i = lanes.indexOf(null);
  return i < 0 ? lanes.length : i;
}

export function extendGraph(g, commits) {
  for (const c of commits) {
    const lanes = g.lanes;
    const pass = [];
    for (let j = 0; j < lanes.length; j++) if (lanes[j] && lanes[j] !== c.hash) pass.push({ lane: j, color: g.colors[j] });
    let col = lanes.indexOf(c.hash);
    const tip = col < 0;
    if (tip) {
      col = free(lanes);
      lanes[col] = c.hash;
      g.colors[col] = g.next++ % COLORS;
    }
    const color = g.colors[col];
    const joins = [];
    for (let j = 0; j < lanes.length; j++) {
      if (j !== col && lanes[j] === c.hash) {
        joins.push({ from: j, color: g.colors[j] });
        lanes[j] = null;
      }
    }
    const parents = c.parents || [];
    lanes[col] = parents[0] || null;
    const outs = [];
    if (parents.length) outs.push({ to: col, color });
    for (let p = 1; p < parents.length; p++) {
      let k = lanes.indexOf(parents[p]);
      if (k < 0) {
        k = free(lanes);
        lanes[k] = parents[p];
        g.colors[k] = g.next++ % COLORS;
      }
      outs.push({ to: k, color: g.colors[k] });
    }
    let width = col + 1;
    for (const p of pass) width = Math.max(width, p.lane + 1);
    for (const j of joins) width = Math.max(width, j.from + 1);
    for (const o of outs) width = Math.max(width, o.to + 1);
    while (lanes.length && lanes[lanes.length - 1] == null) lanes.pop();
    g.rows.push({ col, color, tip, joins, outs, pass, merge: parents.length > 1, root: !parents.length, width });
    g.width = Math.max(g.width, width);
  }
}

export const graphWidth = (lanes) => PAD * 2 + (Math.max(1, lanes) - 1) * LANE_W;
const x = (i) => PAD + i * LANE_W;

// One row as an SVG string. `H` is the row height in px.
export function rowSvg(r, H, lanes) {
  const W = graphWidth(lanes);
  const mid = H / 2;
  const R = r.merge ? 3.25 : 3.5;
  const top = mid - R, bot = mid + R;
  const cx = x(r.col);
  let s = `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" aria-hidden="true">`;
  const line = (c, d) => (s += `<path class="gl c${c}" d="${d}"/>`);
  const curve = (c, x1, y1, x2, y2) => {
    const m = (y1 + y2) / 2;
    line(c, `M${x1} ${y1}C${x1} ${m} ${x2} ${m} ${x2} ${y2}`);
  };
  for (const p of r.pass) if (p.lane < lanes) line(p.color, `M${x(p.lane)} 0V${H}`);
  if (!r.tip) line(r.color, `M${cx} 0V${top}`);
  for (const j of r.joins) curve(j.color, x(j.from), 0, cx, top);
  for (const o of r.outs) {
    if (o.to === r.col) line(o.color, `M${cx} ${bot}V${H}`);
    else curve(o.color, cx, bot, x(o.to), H);
  }
  s += r.merge
    ? `<circle class="gd hollow c${r.color}" cx="${cx}" cy="${mid}" r="${R}"/>`
    : `<circle class="gd c${r.color}" cx="${cx}" cy="${mid}" r="${R}"/>`;
  return s + "</svg>";
}

// A lone dot (used while filtering, where lanes would mislead).
export function dotSvg(color, H, merge) {
  const W = graphWidth(1);
  const mid = H / 2;
  return `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" aria-hidden="true"><circle class="gd${merge ? " hollow" : ""} c${color}" cx="${x(0)}" cy="${mid}" r="${merge ? 3.25 : 3.5}"/></svg>`;
}
