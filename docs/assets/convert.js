const fs = require('fs');

const files = [
  'safety-layer-overview.excalidraw',
  'secrets-overview.excalidraw', 
  'security-data-flow.excalidraw',
  'ironclaw-architecture.excalidraw',
  'ironclaw-security.excalidraw'
];

for (const file of files) {
  const data = JSON.parse(fs.readFileSync(file, 'utf8'));
  
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  
  for (const el of data.elements || []) {
    if (el.isDeleted) continue;
    const x = el.x || 0;
    const y = el.y || 0;
    const w = el.width || 0;
    const h = el.height || 0;

    if (el.type === 'arrow' || el.type === 'line') {
      for (const [px, py] of el.points || []) {
        minX = Math.min(minX, x + px);
        minY = Math.min(minY, y + py);
        maxX = Math.max(maxX, x + px);
        maxY = Math.max(maxY, y + py);
      }
    } else {
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x + w);
      maxY = Math.max(maxY, y + h);
    }
  }

  const padding = 50;
  const width = maxX - minX + padding * 2;
  const height = maxY - minY + padding * 2;

  let svg = `<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="${minX - padding} ${minY - padding} ${width} ${height}">\n  <rect width="100%" height="100%" fill="#ffffff"/>\n  <defs>\n    <marker id="arrowhead" markerWidth="10" markerHeight="10" refX="9" refY="3" orient="auto">\n      <polygon points="0 0, 10 3, 0 6" fill="#1e3a5f" />\n    </marker>\n  </defs>\n`;

  const escapeText = (text) => text.replace(/[&<>]/g, c => ({'&': '&amp;', '<': '&lt;', '>': '&gt;'}[c]));
  const getStroke = (el) => el.strokeColor || '#1e3a5f';
  const getFill = (el) => el.backgroundColor || 'transparent';

  for (const el of data.elements || []) {
    if (el.isDeleted) continue;

    if (el.type === 'rectangle') {
      const rx = el.roundness?.type === 3 ? 8 : 0;
      svg += `  <rect x="${el.x}" y="${el.y}" width="${el.width}" height="${el.height}" fill="${getFill(el)}" stroke="${getStroke(el)}" stroke-width="${el.strokeWidth || 2}" rx="${rx}"/>\n`;
    } else if (el.type === 'ellipse') {
      const cx = el.x + el.width / 2;
      const cy = el.y + el.height / 2;
      const rx = el.width / 2;
      const ry = el.height / 2;
      svg += `  <ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" fill="${getFill(el)}" stroke="${getStroke(el)}" stroke-width="${el.strokeWidth || 2}"/>\n`;
    } else if (el.type === 'diamond') {
      const cx = el.x + el.width / 2;
      const cy = el.y + el.height / 2;
      const points = `${cx},${el.y} ${el.x + el.width},${cy} ${cx},${el.y + el.height} ${el.x},${cy}`;
      svg += `  <polygon points="${points}" fill="${getFill(el)}" stroke="${getStroke(el)}" stroke-width="${el.strokeWidth || 2}"/>\n`;
    } else if (el.type === 'arrow') {
      const points = el.points || [];
      if (points.length >= 2) {
        const d = points.map((p, i) => `${i === 0 ? 'M' : 'L'} ${el.x + p[0]} ${el.y + p[1]}`).join(' ');
        const strokeDash = el.strokeStyle === 'dashed' ? ' stroke-dasharray="5,5"' : '';
        svg += `  <path d="${d}" fill="none" stroke="${getStroke(el)}" stroke-width="${el.strokeWidth || 2}"${strokeDash} marker-end="url(#arrowhead)"/>\n`;
      }
    } else if (el.type === 'line') {
      const points = el.points || [];
      if (points.length >= 2) {
        const d = points.map((p, i) => `${i === 0 ? 'M' : 'L'} ${el.x + p[0]} ${el.y + p[1]}`).join(' ');
        svg += `  <path d="${d}" fill="none" stroke="${getStroke(el)}" stroke-width="${el.strokeWidth || 2}"/>\n`;
      }
    } else if (el.type === 'text') {
      const fontSize = el.fontSize || 16;
      const text = escapeText(el.text || '');
      const lines = text.split('\n');
      const lineHeight = el.lineHeight || 1.25;
      const yOffset = el.verticalAlign === 'middle' ? (lines.length - 1) * fontSize * lineHeight / 2 : 0;

      for (let i = 0; i < lines.length; i++) {
        const y = el.y + (i * fontSize * lineHeight) + fontSize - yOffset;
        const anchor = el.textAlign === 'center' ? 'middle' : 'start';
        const x = el.textAlign === 'center' ? el.x + el.width / 2 : el.x;
        svg += `  <text x="${x}" y="${y}" font-family="sans-serif" font-size="${fontSize}" fill="${el.strokeColor || '#000'}" text-anchor="${anchor}">${lines[i]}</text>\n`;
      }
    }
  }

  svg += '</svg>';
  const outputPath = file.replace('.excalidraw', '.svg');
  fs.writeFileSync(outputPath, svg);
  console.log(`Converted ${file} to ${outputPath}`);
}
