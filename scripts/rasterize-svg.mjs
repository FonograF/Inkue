// Rasterize an SVG into a square transparent PNG.
//
// Usage:
//   node scripts/rasterize-svg.mjs <input.svg> <output.png> [size=1024]
//
// Renders at a high density then downsamples (supersampling) for crisp edges.

import { readFileSync } from "node:fs";
import sharp from "sharp";

const [input, output, sizeArg] = process.argv.slice(2);

if (!input || !output) {
  console.error("Usage: node scripts/rasterize-svg.mjs <input.svg> <output.png> [size]");
  process.exit(1);
}

const size = sizeArg ? Number(sizeArg) : 1024;
const density = Math.max(72, Math.round(size * 0.75));

const svg = readFileSync(input);

await sharp(svg, { density })
  .resize(size, size, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
  .png()
  .toFile(output);

console.log(`Wrote ${output} (${size}x${size})`);
