// Renders an SVG to a PNG with resvg (used for the app icon and README art).
//   node scripts/render-svg.mjs <input.svg> <output.png> [width]
import { readFileSync, writeFileSync } from 'node:fs';
import { Resvg } from '@resvg/resvg-js';

const [input, output, width] = process.argv.slice(2);
if (!input || !output) {
  console.error('usage: node scripts/render-svg.mjs <input.svg> <output.png> [width]');
  process.exit(1);
}
const svg = readFileSync(input, 'utf8');
const resvg = new Resvg(svg, {
  fitTo: width ? { mode: 'width', value: Number(width) } : { mode: 'original' },
  background: 'rgba(0,0,0,0)',
});
writeFileSync(output, resvg.render().asPng());
console.log(`wrote ${output}`);
