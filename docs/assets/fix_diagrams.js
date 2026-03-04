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
  
  // Find all rectangles and their text labels
  const rects = data.elements.filter(e => e.type === 'rectangle');
  const texts = data.elements.filter(e => e.type === 'text');
  
  for (const rect of rects) {
    // Find text that belongs to this rectangle
    const boundText = texts.find(t => t.containerId === rect.id);
    
    if (boundText) {
      // Fix bound text - center it properly within the container
      boundText.x = rect.x + rect.width / 2;
      boundText.width = Math.min(boundText.width, rect.width - 20);
      boundText.textAlign = 'center';
      boundText.verticalAlign = 'middle';
      // Recenter vertically
      boundText.y = rect.y + rect.height / 2 - (boundText.height / 2) + 5;
    }
  }
  
  // Fix free-floating description text elements
  for (const text of texts) {
    if (text.containerId === null && (text.id.includes('desc') || text.id.includes('detail'))) {
      // Find nearest rectangle above
      const nearestRect = rects.filter(r => r.y < text.y).sort((a, b) => b.y - a.y)[0];
      if (nearestRect) {
        text.x = nearestRect.x + nearestRect.width / 2;
        text.width = nearestRect.width - 40;
        text.textAlign = 'center';
      }
    }
  }
  
  fs.writeFileSync(file, JSON.stringify(data, null, 2));
  console.log(`Fixed ${file}`);
}
