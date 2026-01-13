# Implementation Plan for Mermaid Rendering in Nebula Chat

## Overview

Adding Mermaid diagram rendering capability to Nebula Chat would enhance its ability to visualize complex information structures, workflows, and relationships discussed in conversations.

## Implementation Approach

### 1. Frontend Changes (Primary Work)

Based on the existing code in `src/components/ChatInterface.tsx`, Nebula already uses:
- `react-markdown` for markdown processing
- Custom components for various markdown elements
- Syntax highlighting with `react-syntax-highlighter`

#### Required Changes:

1. **Install Mermaid.js dependency**:
   ```bash
   npm install mermaid
   ```

2. **Modify the ChatInterface component** to detect and render Mermaid diagrams:
   
   In `src/components/ChatInterface.tsx`, we'd need to:
   - Add Mermaid detection logic in the markdown processing pipeline
   - Create a custom component for Mermaid diagrams
   - Add useEffect hook to initialize and render diagrams

3. **Add Mermaid component**:
   ```tsx
   import mermaid from 'mermaid';
   
   // Initialize Mermaid
   useEffect(() => {
     mermaid.initialize({ 
       startOnLoad: true,
       theme: 'dark', // Match the app theme
       // Other configuration options
     });
   }, []);
   
   // Custom component for Mermaid diagrams
   const MermaidDiagram = ({ chart }) => {
     const ref = useRef<HTMLDivElement>(null);
     
     useEffect(() => {
       if (ref.current) {
         mermaid.render('mermaid-diagram-' + Math.random(), chart)
           .then(({ svg, bindFunctions }) => {
             if (ref.current) {
               ref.current.innerHTML = svg;
               bindFunctions?.(ref.current);
             }
           })
           .catch((error) => {
             console.error('Mermaid rendering error:', error);
           });
       }
     }, [chart]);
     
     return <div ref={ref} className="mermaid-container" />;
   };
   ```

4. **Update markdown processing** to detect Mermaid code blocks:
   ```tsx
   // In the ReactMarkdown components configuration
   components: {
     // ... existing components
     code: ({ node, inline, className, children, ...props }) => {
       const match = /language-(\w+)/.exec(className || '');
       
       // Handle Mermaid diagrams
       if (!inline && match && match[1] === 'mermaid') {
         return <MermaidDiagram chart={String(children).replace(/\n$/, '')} />;
       }
       
       // Existing code highlighting logic
       // ...
     }
   }
   ```

### 2. Backend Considerations

The backend shouldn't require significant changes since:
- Mermaid diagrams would be sent as markdown code blocks (\`\`\`mermaid)
- The backend treats these as regular message content
- No special processing needed in the Rust code

### 3. Styling Updates

Add CSS for proper diagram rendering in `src/App.css` or relevant CSS files:
```css
.mermaid-container {
  display: flex;
  justify-content: center;
  margin: 1rem 0;
  overflow-x: auto;
  padding: 1rem;
  background: var(--bg-secondary);
  border-radius: 8px;
}

.mermaid-container svg {
  max-width: 100%;
  height: auto;
}
```

### 4. Theme Integration

Since Nebula supports multiple themes (dark, light, solarized), we'd need to:
- Configure Mermaid to use the appropriate theme based on app settings
- Listen to theme changes and re-render diagrams when theme changes

### 5. Performance Considerations

- Add debouncing to prevent excessive re-renders
- Consider lazy loading for complex diagrams
- Add error boundaries to handle malformed Mermaid syntax

### 6. Testing

Following the smoke test checklist in `docs/dev_smoke_test.md`, we'd need to verify:
1. Basic Mermaid diagram renders correctly
2. Diagram updates when message is edited
3. Works with different themes
4. Proper error handling for invalid syntax
5. Performance with complex diagrams

## Estimated Effort

This would be a moderate-sized feature requiring approximately:
- 2-3 days for implementation
- 1 day for testing across different scenarios
- 1 day for documentation and examples

The implementation would primarily be in the frontend React components, with minimal backend changes required. The existing markdown infrastructure provides a solid foundation for integrating Mermaid diagram rendering.