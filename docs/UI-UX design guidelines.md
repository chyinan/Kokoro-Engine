# Kokoro Engine — UI/UX Design Guidelines

> **Version:** 2.0  
> **Last Updated:** 2026-02-12  
> **Status:** Canonical — all frontend components and layouts **must** follow this document.  
> **Companion:** [architecture.md](file:///d:/Program/Kokoro%20Engine/docs/architecture.md) · [PRD.md](file:///d:/Program/Kokoro%20Engine/docs/PRD.md)

---

## 1. Design Philosophy

### 1.1 Core Identity

Kokoro Engine is a **virtual character companion** — not a productivity tool, not an admin panel.  
Every pixel must reinforce the feeling of being **in a character's world**.

| Keyword | What It Means for UI |
|---|---|
| **Warm** | Soft edges, gentle animations, inviting tones, nothing feels cold or clinical |
| **Immersive** | The character's world extends into the UI — panels float over the character, not the other way around |
| **Character-centric** | The Live2D avatar is always the emotional focus; UI is secondary, supportive |
| **Calm** | Low visual noise, generous whitespace, breathing room in every layout |
| **Personal** | Feels like *your* companion app, not a mass-market SaaS product |

> **Guiding Mantra:**  
> *"The user is interacting with a character, not operating software."*

### 1.2 Anti-Patterns — Actively Avoid

| ❌ Don't | ✅ Do Instead |
|---|---|
| Cold enterprise SaaS aesthetics | Warm, companion-app feel |
| Dense data tables or dashboards | Spacious, breathing layouts |
| Overly technical / admin-style layouts | Visual novel / character app style |
| Flat, corporate color schemes | Deep, atmospheric, character-themed palettes |
| Modal-heavy workflows | Inline, contextual interactions |
| Hard borders and sharp dividers | Soft shadows, glassmorphism, gradient separators |
| System-font defaults | Curated typography with personality |

### 1.3 Design Pillars (Ordered by Priority)

1. **Emotion over information** — Prioritize how the UI *feels* over how much data it shows
2. **Character first** — The avatar is always the hero; UI panels are supporting cast
3. **Modular & themeable** — Every visual decision must flow through tokens, never hardcoded
4. **Graceful degradation** — Offline, error, and loading states must feel warm, not broken
5. **Creator-friendly** — Mod authors can override any visual layer without breaking core UX

---

## 2. Visual Style Direction

### 2.1 Design Style: "Neon Glass"

The default visual style is **Neon Glass** — a cinematic, atmospheric aesthetic that combines:

- Deep, near-black backgrounds with subtle warmth
- Translucent glass-like panels with backdrop blur
- A single vivid accent color (cyan) used sparingly for focus and interactivity
- Subtle noise/grain texture for atmospheric depth
- Smooth, spring-based animations

> Think: **Sci-fi visual novel UI** — not cyberpunk harshness, but elegant ethereal glow.

### 2.2 Color System

#### Primary Palette

| Token | Hex | Role | Usage |
|---|---|---|---|
| `--color-bg-primary` | `#050510` | App background | Root-level background |
| `--color-bg-surface` | `rgba(10, 10, 20, 0.6)` | Panel backgrounds | Glassmorphism surfaces |
| `--color-bg-overlay` | `rgba(0, 0, 0, 0.4)` | Overlay / dimming | Modal backdrops, input fields |
| `--color-bg-elevated` | `rgba(15, 15, 30, 0.8)` | Elevated surfaces | Dropdowns, tooltips, popovers |

#### Text Colors

| Token | Hex | Role | Contrast Ratio vs `#050510` |
|---|---|---|---|
| `--color-text-primary` | `#e0e6ed` | Main body text | ≥ 15:1 ✅ |
| `--color-text-secondary` | `#94a3b8` | Secondary / labels | ≥ 7:1 ✅ |
| `--color-text-muted` | `#64748b` | Metadata / timestamps | ≥ 4.5:1 ✅ |

#### Accent & Semantic Colors

| Token | Hex | Role | Glow Shadow |
|---|---|---|---|
| `--color-accent` | `#00f0ff` | Primary accent, CTA buttons, links | `0 0 10px rgba(0, 240, 255, 0.5)` |
| `--color-accent-hover` | `#33f5ff` | Hover state | `0 0 14px rgba(0, 240, 255, 0.6)` |
| `--color-accent-active` | `#00d4e0` | Active / pressed state | `0 0 6px rgba(0, 240, 255, 0.3)` |
| `--color-accent-subtle` | `rgba(0, 240, 255, 0.10)` | Accent tint backgrounds | — |
| `--color-success` | `#10b981` | Success states, online indicators | `0 0 8px rgba(16, 185, 129, 0.5)` |
| `--color-warning` | `#f59e0b` | Warnings, caution states | — |
| `--color-error` | `#ef4444` | Errors, destructive actions | — |

#### Surface & Border

| Token | Value | Role |
|---|---|---|
| `--color-border` | `rgba(255, 255, 255, 0.08)` | Panel borders, dividers |
| `--color-border-accent` | `rgba(0, 240, 255, 0.30)` | Focus rings, active borders |
| `--color-border-subtle` | `rgba(255, 255, 255, 0.04)` | Very subtle separators |

#### Character Mood Colors (Dynamic)

The accent color can shift based on character mood (`CharacterState.mood: 0.0 → 1.0`):

| Mood Range | Color Shift | Effect |
|---|---|---|
| `0.0 – 0.2` (Sad) | `#8b9cf7` (soft lavender) | Cooler, subdued ambient glow |
| `0.2 – 0.4` (Melancholy) | `#a78bfa` (muted violet) | Slight purple shift |
| `0.4 – 0.6` (Neutral) | `#00f0ff` (default cyan) | Standard accent |
| `0.6 – 0.8` (Happy) | `#34d399` (warm teal) | Warmer, brighter glow |
| `0.8 – 1.0` (Joyful) | `#fbbf24` (soft gold) | Warm, radiant glow |

> **Implementation:** Interpolate `--color-accent` based on `CharacterState.mood` in `ThemeProvider`. The mood color should transition smoothly over `500ms`.

### 2.3 Surface Treatment

| Property | Specification |
|---|---|
| **Glass panels** | `background: var(--color-bg-surface)` + `backdrop-filter: blur(12px)` |
| **Gradients** | Soft, 2-stop max; never sharp color breaks |
| **Corners** | `border-radius: 12px` for panels, `8px` for buttons, `6px` for inputs |
| **Borders** | `1px solid var(--color-border)` — minimal, only where needed |
| **Shadows** | Soft, diffused: `0 8px 32px rgba(0, 0, 0, 0.3)` |
| **Noise overlay** | SVG noise at `opacity: 0.05`, `mix-blend-mode: overlay`, `z-index: 50` |

### 2.4 Typography

| Level | Font | Size | Weight | Line-Height |
|---|---|---|---|---|
| **Hero / App Title** | Rajdhani | `1.75rem` (28px) | 700 | 1.2 |
| **Section Header** | Rajdhani | `1.25rem` (20px) | 600 | 1.3 |
| **Panel Title** | Rajdhani | `1rem` (16px) | 600 | 1.3 |
| **Body** | Quicksand | `0.9375rem` (15px) | 400 | 1.6 |
| **UI Labels** | Quicksand | `0.8125rem` (13px) | 500 | 1.4 |
| **Metadata** | Quicksand | `0.75rem` (12px) | 400 | 1.4 |
| **Code / Technical** | `'JetBrains Mono', monospace` | `0.8125rem` | 400 | 1.5 |

```css
/* Typography Token Scale */
--font-heading:     'Rajdhani', sans-serif;
--font-body:        'Quicksand', sans-serif;
--font-mono:        'JetBrains Mono', 'Fira Code', monospace;

--text-hero:        700 1.75rem/1.2 var(--font-heading);
--text-h2:          600 1.25rem/1.3 var(--font-heading);
--text-h3:          600 1rem/1.3 var(--font-heading);
--text-body:        400 0.9375rem/1.6 var(--font-body);
--text-label:       500 0.8125rem/1.4 var(--font-body);
--text-meta:        400 0.75rem/1.4 var(--font-body);

/* Tracking */
--tracking-wide:    0.05em;   /* Headings, panel titles */
--tracking-wider:   0.1em;    /* Status labels, uppercase text */
```

> **Rule:** Headings use `Rajdhani` with `letter-spacing: var(--tracking-wide)` and `text-transform: uppercase` for the cyberpunk UI feel. Body text uses `Quicksand` for warmth and readability.

### 2.5 Iconography

| Guideline | Detail |
|---|---|
| **Library** | [Lucide React](https://lucide.dev/) — consistent, clean line icons |
| **Size** | `16px` inline, `18px` buttons, `24px` standalone |
| **Stroke** | `1.5px` default — match the thin, elegant aesthetic |
| **Color** | Inherit from parent text color; accent color for interactive icons |
| **Animation** | Subtle scale on hover (`1.1x`), never rotate or bounce |

---

## 3. Layout System & Responsive Strategy

### 3.1 Core Layout: Layer + Grid

The layout uses two conceptual layers stacked via `position: absolute`:

```
┌──────────────────────────────────────────────────────────┐
│                      Tauri Window                        │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │  Layer 0: Live2D Stage (full bleed, z-index: 0)    │  │
│  │  ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐  │  │
│  │  │           ★ Character (visual focus)         │  │  │
│  │  │           Always generous space              │  │  │
│  │  │           60fps PixiJS rendering             │  │  │
│  │  └ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┘  │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │  Layer 1: UI Grid Overlay (z-index: 10)            │  │
│  │  pointer-events: none (click-through by default)   │  │
│  │                                                    │  │
│  │  ┌──────────┬────────────────┬─────────────┐      │  │
│  │  │  header   │    header      │   header    │      │  │
│  │  ├──────────┼────────────────┼─────────────┤      │  │
│  │  │ ChatPanel│    (empty)     │  Sidebar    │      │  │
│  │  │ 350px    │   click-thru   │  300px      │      │  │
│  │  ├──────────┼────────────────┼─────────────┤      │  │
│  │  │  footer   │    footer      │   footer    │      │  │
│  │  └──────────┴────────────────┴─────────────┘      │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │  Layer 2: Noise Overlay (z-index: 50)              │  │
│  │  pointer-events: none, opacity: 0.05               │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │  Layer 3: Modals / Toasts (z-index: 100+)          │  │
│  └────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

### 3.2 Default Grid Configuration

```css
/* UI Overlay Grid — defined in LayoutConfig JSON */
grid-template-columns: 350px 1fr 300px;
grid-template-rows:    60px 1fr 60px;
grid-template-areas:
  "header    header  header"
  "highlight main    sidebar"
  "footer    footer  footer";
```

| Area | Component | Purpose |
|---|---|---|
| `highlight` | `ChatPanel` | Primary interaction — chat with character |
| `sidebar` | `ModList` / Settings | Secondary controls |
| `header` | *(reserved)* | Status bar, character name, mood indicator |
| `footer` | *(reserved)* | Quick actions, expression buttons |
| `main` | *(empty / click-through)* | Allows interaction with Live2D stage behind |

### 3.3 Z-Index Discipline

| Layer | Z-Index | Content | `pointer-events` |
|---|---|---|---|
| **Live2D Stage** | `0` | Character rendering (PixiJS canvas) | `auto` (captures gaze/click) |
| **UI Grid** | `10` | Chat, sidebar, header, footer | `none` on grid, `auto` on panels |
| **Noise Overlay** | `50` | Atmospheric grain texture | `none` |
| **Tooltips** | `80` | Contextual help | `auto` |
| **Modals** | `100` | Settings, confirmations, persona editor | `auto` |
| **Toasts** | `110` | Notifications, errors | `auto` |
| **System Overlay** | `999` | Fatal error / loading screen | `auto` |

> **Rule:** Never use arbitrary z-index values. Always reference this table. Mods must stay within `z: 10–49`.

### 3.4 Responsive Breakpoints

Kokoro Engine is a **desktop Tauri app** with resizable windows. The responsive strategy targets **window size**, not device type.

| Breakpoint | Width | Layout Behavior |
|---|---|---|
| `--bp-compact` | `< 768px` | Single-column: panels stack, sidebar hidden |
| `--bp-standard` | `768px – 1199px` | Two-column: chat + character, sidebar collapsed |
| `--bp-wide` | `1200px – 1599px` | Full three-column grid (default) |
| `--bp-ultrawide` | `≥ 1600px` | Widen character area, cap panel max-width |

#### Compact Mode (`< 768px`)

```css
grid-template-columns: 1fr;
grid-template-rows:    50px 1fr auto;
grid-template-areas:
  "header"
  "main"
  "highlight";
```

- Character fills the entire background
- Chat panel becomes a bottom sheet (40% height, expandable)
- Sidebar is accessible via a hamburger menu overlay
- Header shrinks to minimal status bar

#### Standard Mode (`768px – 1199px`)

```css
grid-template-columns: 320px 1fr;
grid-template-rows:    60px 1fr 60px;
grid-template-areas:
  "header    header"
  "highlight main"
  "footer    footer";
```

- Two-column: chat + character
- Sidebar collapses to an expandable drawer from the right edge
- Chat panel width reduces to `320px`

#### Wide Mode (`1200px – 1599px`) — Default

Full three-column grid as defined in Section 3.2.

#### Ultrawide Mode (`≥ 1600px`)

```css
grid-template-columns: 380px 1fr 320px;
/* Max content width capped at 1600px, centered */
```

- Panel widths increase slightly for comfort
- Character area gets extra breathing room
- Content area capped at `max-width: 1600px` and centered

### 3.5 Panel Behavior Rules

| Rule | Detail |
|---|---|
| **Float over character** | Panels overlay the character — never push or crop the Live2D canvas |
| **Resizable** | Panels support drag-to-resize via edge handles |
| **Collapsible** | Every panel can collapse to a minimal icon/tab |
| **Margin** | All panels have `margin: 20px` from window edges |
| **Max height** | Panels never exceed `calc(100vh - 120px)` |
| **Scroll** | Internal scroll only — panels never cause page scroll |
| **Semi-transparent** | Panels must show the character blurred behind them |

### 3.6 Spacing Scale

All spacing uses a **4px base unit**. Never use arbitrary pixel values.

```css
--space-1:   4px;     /* Tight — icon gaps, inline padding */
--space-2:   8px;     /* Compact — between related items */
--space-3:  12px;     /* Snug — form element padding */
--space-4:  16px;     /* Standard — section padding, card padding */
--space-5:  20px;     /* Comfortable — panel margins */
--space-6:  24px;     /* Spacious — between sections */
--space-8:  32px;     /* Generous — major separations */
--space-10: 40px;     /* Maximum — hero spacers */
--space-12: 48px;     /* Extra — top-level layout gaps */
```

> **Tailwind mapping:** `p-1` = 4px, `p-2` = 8px, `p-3` = 12px, `p-4` = 16px, `p-5` = 20px, `gap-6` = 24px, etc.

---

## 4. Interaction Design & Animation

### 4.1 Animation Philosophy

Animations in Kokoro Engine serve one purpose: **making the character world feel alive**.

- Animations are **purposeful** — every motion communicates state change
- Never animate for decoration alone
- Prefer **spring-based** physics over linear easing for organic feel
- All animations must respect `prefers-reduced-motion`

### 4.2 Timing Tokens

```css
/* Duration Tokens */
--duration-instant:   0ms;        /* Reduced-motion override */
--duration-fast:      150ms;      /* Micro-interactions: hover, focus ring */
--duration-normal:    300ms;      /* Panel slide, fade-in, message entry */
--duration-slow:      500ms;      /* Major state: mood shift, theme switch */
--duration-dramatic:  800ms;      /* Hero transitions: character swap, scene change */

/* Easing Tokens */
--ease-out:           cubic-bezier(0.16, 1, 0.3, 1);    /* Default — decelerate in */
--ease-in-out:        cubic-bezier(0.65, 0, 0.35, 1);   /* Symmetrical — panels */
--ease-spring:        cubic-bezier(0.34, 1.56, 0.64, 1); /* Springy — button press */
```

### 4.3 Framer Motion Presets

All motion presets are defined in `ThemeConfig.animations` and consumed by `LayoutRenderer`:

```typescript
// In ThemeConfig.animations
panelEntry: {
  initial:    { opacity: 0, x: -20, scale: 0.95 },
  animate:    { opacity: 1, x: 0,   scale: 1 },
  exit:       { opacity: 0, x: -20, scale: 0.95 },
  transition: { type: "spring", stiffness: 300, damping: 30 }
},
messageEntry: {
  initial:    { opacity: 0, y: 10, scale: 0.95 },
  animate:    { opacity: 1, y: 0,  scale: 1 },
  transition: { duration: 0.3 }
},
moodShift: {
  transition: { duration: 0.5, ease: "easeInOut" }
},
modalOverlay: {
  initial:    { opacity: 0 },
  animate:    { opacity: 1 },
  exit:       { opacity: 0 },
  transition: { duration: 0.2 }
}
```

### 4.4 Feedback — Every Action Gets a Response

| User Action | Visual Feedback | Audio Feedback |
|---|---|---|
| Send message | Input clears, message slides in with `messageEntry` | Optional soft UI sound |
| Receive AI text | Text streams token-by-token; typing indicator pulses | — |
| Switch expression | Live2D blends smoothly; optional glow particle effect | — |
| Voice playing | Pulsing glow ring around character; lip-sync animation | Voice audio plays |
| AI thinking | Breathing pulse animation on character; subtle ellipsis | — |
| Error | Gentle toast slides in from top-right; warm red, not alarming | — |
| Hover button | Scale `1.05x` + accent glow shadow | — |
| Press button | Scale `0.95x` + darker accent | — |
| Panel collapse | Smooth width → 0 with fade | — |
| Mood change | Accent color smoothly transitions; ambient glow shifts | — |

### 4.5 State-Specific Visual Indicators

#### Character Speaking State

```
┌─────────────────────────────────┐
│                                 │
│     🔊 Speaking Indicator       │
│                                 │
│  • Pulsing glow ring around     │
│    character at accent color    │
│  • Ring opacity: 0.3 → 0.7     │
│  • Pulse period: 1.2s          │
│  • Synced with audio amplitude  │
│    when lip-sync available      │
│                                 │
└─────────────────────────────────┘
```

#### AI Thinking / Loading

```
┌─────────────────────────────────┐
│                                 │
│     💭 Thinking Indicator       │
│                                 │
│  • Subtle breathing animation   │
│    on character (scale 1.0→1.01)│
│  • Animated ellipsis "..." in   │
│    chat panel                   │
│  • Muted pulse on accent glow   │
│  • Duration: until first token  │
│                                 │
└─────────────────────────────────┘
```

#### Offline / Degraded State

```
┌─────────────────────────────────┐
│                                 │
│     📴 Offline Indicator        │
│                                 │
│  • Status dot: amber (#f59e0b)  │
│  • Header text: "OFFLINE"       │
│  • Chat input placeholder:      │
│    "AI unavailable — offline"   │
│  • Character still renders and  │
│    responds to gaze/click       │
│                                 │
└─────────────────────────────────┘
```

### 4.6 Scroll Behavior

| Context | Behavior |
|---|---|
| **Chat messages** | Auto-scroll to bottom on new message; cancel auto-scroll if user scrolls up |
| **Scrollbar style** | Thin (`4px`), accent-colored thumb, transparent track |
| **Inertia** | Smooth scroll with `scroll-behavior: smooth` |
| **Overscroll** | `overscroll-behavior: contain` — prevent page bounce |

```css
/* Scrollbar Styling */
.scrollable {
  scrollbar-width: thin;
  scrollbar-color: var(--color-accent) transparent;
}
.scrollable::-webkit-scrollbar { width: 4px; }
.scrollable::-webkit-scrollbar-thumb {
  background: var(--color-accent);
  border-radius: 2px;
}
.scrollable::-webkit-scrollbar-track { background: transparent; }
```

### 4.7 Reduced Motion

**All animations must gracefully degrade** when `prefers-reduced-motion: reduce` is active:

```css
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0ms !important;
    animation-delay: 0ms !important;
    transition-duration: 0ms !important;
    transition-delay: 0ms !important;
  }
}
```

> **Framer Motion:** Use `useReducedMotion()` hook to conditionally disable spring animations. Replace with instant state changes.

---

## 5. Component Discipline

### 5.1 Component Architecture Rules

| Rule | Detail |
|---|---|
| **Registry-first** | All UI components register via `ComponentRegistry` by name |
| **Token-only styling** | Use CSS custom properties and Tailwind utilities — no inline hex/rgb |
| **Semantic naming** | Component names describe *what*, not *how*: `ChatPanel`, not `LeftSidebar` |
| **Single responsibility** | Each component does one thing well |
| **Props over state** | Prefer props for configuration; use state only for interactive behavior |
| **Consistent spacing** | Always use spacing tokens (`--space-*` or Tailwind `p-*`, `gap-*`) |
| **No inline CSS** | No `style={{}}` for visual properties — exceptions only for dynamic layout (`gridArea`, `zIndex`) |

### 5.2 Component Catalog & Specs

#### `ChatPanel`

| Property | Specification |
|---|---|
| **Location** | `src/ui/widgets/ChatPanel.tsx` |
| **Grid area** | `highlight` (left column) |
| **Width** | `350px` (wide), `320px` (standard), `100%` (compact) |
| **Background** | `var(--color-bg-surface)` with `backdrop-filter: blur(var(--glass-blur))` |
| **Border** | `1px solid var(--color-border)` on right edge |
| **Sections** | Header → Messages → Input (flex column, `h-full`) |
| **Header** | Panel title in `Rajdhani`, accent color, `tracking-widest`, uppercase |
| **Messages** | `flex-1 overflow-y-auto`, spacing `space-y-4` between bubbles |
| **User bubble** | Right-aligned, `bg-accent/10`, `border-accent/30`, `text-accent` |
| **AI bubble** | Left-aligned, `bg-slate-900/50`, `border-slate-700/50`, `text-slate-300` |
| **Input** | `bg-black/40`, `border-[var(--color-border)]`, focus: accent border + glow |
| **Send button** | `bg-accent`, `text-black`, hover: `bg-white`, scale `1.1x` on hover |

#### `Live2DStage`

| Property | Specification |
|---|---|
| **Location** | `src/features/live2d/Live2DViewer.tsx` |
| **Layer** | `z-index: 0`, `position: absolute`, `inset: 0` |
| **Rendering** | PixiJS canvas, full viewport, 60fps |
| **Pointer events** | `auto` — captures mouse for gaze tracking and hit areas |
| **Background** | Transparent — app background shows through |
| **Character position** | Centered, slight offset right to leave room for chat panel |

#### `ModList`

| Property | Specification |
|---|---|
| **Grid area** | `sidebar` (right column) |
| **Width** | `300px` (wide), drawer (standard), overlay (compact) |
| **Items** | Card-style with name, version, description |
| **Card** | `bg-surface`, `rounded-lg`, `p-4`, hover: border-accent subtle |
| **Load button** | Accent-colored, small, right-aligned |

#### Future: `SettingsPanel`

| Property | Specification |
|---|---|
| **Type** | Modal overlay (`z-index: 100`) |
| **Width** | `min(480px, 90vw)` |
| **Background** | `var(--color-bg-elevated)` with strong blur |
| **Sections** | Tabs: API Config, Persona, TTS, Theme |
| **Form inputs** | Consistent: `bg-black/40`, `border-[var(--color-border)]`, accent focus |

### 5.3 Component Styling Rules

#### Do ✅

```tsx
// Using Tailwind + CSS variables (correct)
<div className="bg-[var(--color-bg-surface)] border border-[var(--color-border)]
               backdrop-blur-[var(--glass-blur)] rounded-xl p-4">

// Using clsx for conditional classes (correct)
<div className={clsx(
  "max-w-[85%] p-3 rounded-lg text-sm",
  msg.role === "user"
    ? "ml-auto bg-[var(--color-accent)]/10 text-[var(--color-accent)]"
    : "mr-auto bg-slate-900/50 text-slate-300"
)}>
```

#### Don't ❌

```tsx
// Hardcoded values (wrong)
<div style={{ backgroundColor: "#050510", borderRadius: "12px" }}>

// Random Tailwind values not from scale (wrong)
<div className="p-[13px] gap-[7px] rounded-[11px]">

// Inline hex colors (wrong)
<div className="text-[#e0e6ed] bg-[#0a0a14]">
```

---

## 6. Theming & Mod Support

### 6.1 Theme Token System

All visual properties flow through CSS custom properties set by `ThemeProvider`:

#### Complete Token Catalog

```css
/* ─── Colors ─── */
--color-bg-primary:       #050510;
--color-bg-surface:       rgba(10, 10, 20, 0.6);
--color-bg-overlay:       rgba(0, 0, 0, 0.4);
--color-bg-elevated:      rgba(15, 15, 30, 0.8);

--color-text-primary:     #e0e6ed;
--color-text-secondary:   #94a3b8;
--color-text-muted:       #64748b;

--color-accent:           #00f0ff;
--color-accent-hover:     #33f5ff;
--color-accent-active:    #00d4e0;
--color-accent-subtle:    rgba(0, 240, 255, 0.10);

--color-success:          #10b981;
--color-warning:          #f59e0b;
--color-error:            #ef4444;

--color-border:           rgba(255, 255, 255, 0.08);
--color-border-accent:    rgba(0, 240, 255, 0.30);
--color-border-subtle:    rgba(255, 255, 255, 0.04);

/* ─── Glow Shadows ─── */
--glow-accent:            0 0 10px rgba(0, 240, 255, 0.5);
--glow-accent-hover:      0 0 14px rgba(0, 240, 255, 0.6);
--glow-success:           0 0 8px rgba(16, 185, 129, 0.5);

/* ─── Surfaces ─── */
--radius-sm:              6px;
--radius-md:              8px;
--radius-lg:              12px;
--radius-xl:              16px;
--radius-full:            9999px;

--shadow-sm:              0 2px 8px rgba(0, 0, 0, 0.2);
--shadow-md:              0 4px 16px rgba(0, 0, 0, 0.25);
--shadow-lg:              0 8px 32px rgba(0, 0, 0, 0.3);

--glass-blur:             12px;

/* ─── Typography ─── */
--font-heading:           'Rajdhani', sans-serif;
--font-body:              'Quicksand', sans-serif;
--font-mono:              'JetBrains Mono', monospace;

/* ─── Spacing ─── */
--space-1: 4px;   --space-2: 8px;   --space-3: 12px;
--space-4: 16px;  --space-5: 20px;  --space-6: 24px;
--space-8: 32px;  --space-10: 40px; --space-12: 48px;

/* ─── Timing ─── */
--duration-fast:    150ms;
--duration-normal:  300ms;
--duration-slow:    500ms;

--ease-out:         cubic-bezier(0.16, 1, 0.3, 1);
--ease-spring:      cubic-bezier(0.34, 1.56, 0.64, 1);
```

### 6.2 ThemeConfig Schema

```typescript
interface ThemeConfig {
  id: string;                    // Unique theme ID (e.g., "neon-glass")
  name: string;                  // Display name (e.g., "Neon Glass")
  variables: Record<string, string>;  // CSS custom properties
  assets?: {
    fonts?: string[];            // Google Fonts URLs to inject
    background?: string;         // Background image URL
    noise_texture?: string;      // SVG noise overlay
  };
  animations?: Record<string, MotionAnimationConfig>;
}
```

### 6.3 Theme Switching

Themes swap at runtime via `ThemeProvider.setTheme()`:

1. New `variables` are applied to `document.documentElement.style`
2. New fonts are injected as `<link>` elements
3. Background image is updated on `document.body`
4. All components re-render with new token values automatically
5. Transition: smooth `500ms` crossfade for background, instant for tokens

### 6.4 Mod UI Contract

Mods that provide custom UI **must** follow these rules:

| # | Rule | Enforcement |
|---|---|---|
| 1 | **Use theme tokens** — no hardcoded colors, sizes, or fonts | Review by `ComponentRegistry` |
| 2 | **Register via `ComponentRegistry`** — components must be named and registered | Required for layout inclusion |
| 3 | **Follow spacing scale** — use `--space-*` tokens or Tailwind utilities | Visual audit |
| 4 | **Respect layout zones** — mods render in designated grid areas only | Z-index restricted to `10–49` |
| 5 | **Support reduced motion** — check `prefers-reduced-motion` | Accessibility audit |
| 6 | **Use `mod://` protocol** — for all asset loading | Security enforcement |

### 6.5 Creating a Custom Theme

```typescript
// mods/my-theme/theme.ts
import { ThemeConfig } from "../../src/ui/layout/types";

export const warmSunset: ThemeConfig = {
  id: "warm-sunset",
  name: "Warm Sunset",
  variables: {
    "--color-bg-primary":    "#1a0a1e",
    "--color-bg-surface":    "rgba(26, 10, 30, 0.7)",
    "--color-text-primary":  "#fce4ec",
    "--color-text-secondary":"#ce93d8",
    "--color-accent":        "#ff6f61",
    "--color-accent-hover":  "#ff8a80",
    "--color-border":        "rgba(255, 111, 97, 0.15)",
    "--glass-blur":          "16px",
    "--font-heading":        "'Cinzel', serif",
    "--font-body":           "'Lora', serif",
  },
  assets: {
    fonts: [
      "https://fonts.googleapis.com/css2?family=Cinzel:wght@500;700&family=Lora:wght@400;500&display=swap"
    ],
  },
};
```

---

## 7. Accessibility & Comfort

### 7.1 Minimum Requirements

| Guideline | Specification |
|---|---|
| **Text contrast** | Body text: `≥ 4.5:1` ratio. Large text (≥18px): `≥ 3:1` ratio |
| **Click targets** | Minimum `44px × 44px` for all interactive elements |
| **Focus indicators** | Visible focus ring: `2px solid var(--color-accent)` with `offset: 2px` |
| **No flashing** | No strobing, rapid flashing, or high-frequency animations |
| **Keyboard navigation** | All interactive elements reachable and operable via keyboard |
| **Tab order** | Logical flow: header → chat input → messages → sidebar → footer |
| **Screen reader** | All interactive elements have `aria-label` or visible text |
| **Reduced motion** | Respect `prefers-reduced-motion` (see Section 4.7) |

### 7.2 Long-Session Comfort

Kokoro Engine is designed for **extended use** — users may interact with their character for hours.

| Principle | Implementation |
|---|---|
| **Dark mode default** | Deep, warm backgrounds reduce eye strain |
| **Warm color temperature** | Avoid pure blue light; tint toward warm neutrals |
| **Low visual noise** | Minimal animation when idle; character breathes gently |
| **Consistent brightness** | No sudden brightness changes between states |
| **Generous text size** | Body text minimum `15px` — never below `12px` for any element |
| **Comfortable line-height** | `1.6` for body text ensures readability |

### 7.3 Focus Ring Styling

```css
/* Global focus-visible styling */
:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
  border-radius: var(--radius-sm);
}

/* Remove default outline for mouse users */
:focus:not(:focus-visible) {
  outline: none;
}
```

---

## 8. Frontend Engineer Agent — Coding Prompt

> **Purpose:** This section provides a structured prompt / reference for AI coding agents (and human engineers) when building or modifying Kokoro Engine frontend components. Following this ensures **visual consistency** across all contributors.

### 8.1 System Prompt for Frontend Agent

```
You are building UI for Kokoro Engine — a virtual character companion app.

STYLE IDENTITY: "Neon Glass"
- Deep dark backgrounds (#050510), translucent glass panels
- Cyan accent (#00f0ff) used sparingly for interactive elements
- Rajdhani for headings (uppercase, tracked), Quicksand for body text
- Soft glow shadows, not harsh drop shadows
- The character (Live2D) is always the visual hero

CORE RULES:
1. NEVER use hardcoded colors — always use CSS custom properties (--color-*)
2. NEVER use inline style={{}} for visual properties — only for dynamic layout (gridArea, zIndex)
3. ALWAYS use Tailwind utilities referencing CSS variables: bg-[var(--color-bg-surface)]
4. ALWAYS use the spacing scale: p-1=4px, p-2=8px, p-3=12px, p-4=16px, p-5=20px
5. ALWAYS use clsx() for conditional class composition
6. ALWAYS use framer-motion for animations — no raw CSS transitions on components
7. ALWAYS register components in ComponentRegistry by a semantic name
8. ALWAYS add pointer-events-auto on panels in the UI overlay grid
9. NEVER add page-level scroll — all scroll is internal to panels
10. ALWAYS respect prefers-reduced-motion

COMPONENT STRUCTURE:
- Import: { useState, useRef, useEffect } from "react"
- Import: { motion, AnimatePresence } from "framer-motion"
- Import: { clsx } from "clsx"
- Import: icons from "lucide-react" (16-18px, strokeWidth 1.5)
- Export: default function ComponentName()

PANEL TEMPLATE:
- Outer: flex flex-col h-full w-full
- Background: bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]
- Border: border border-[var(--color-border)]
- Shadow: shadow-lg (use --shadow-lg token)
- Rounded: rounded-xl (12px)

BUTTON PATTERNS:
- Primary: bg-[var(--color-accent)] text-black rounded-lg px-4 py-2
- Hover: whileHover={{ scale: 1.05 }} + hover:bg-white
- Press: whileTap={{ scale: 0.95 }}
- Icon button: p-2 rounded-md

INPUT PATTERNS:
- Background: bg-black/40
- Border: border border-[var(--color-border)]
- Focus: focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]
- Text: text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]
- Font: font-mono for code inputs, font-body for text inputs

MESSAGE BUBBLE PATTERNS:
- User: ml-auto bg-[var(--color-accent)]/10 border-[var(--color-accent)]/30 text-[var(--color-accent)] rounded-lg rounded-tr-none
- AI:   mr-auto bg-slate-900/50 border-slate-700/50 text-slate-300 rounded-lg rounded-tl-none
- Entry: motion.div with { opacity: 0, y: 10, scale: 0.95 } → { opacity: 1, y: 0, scale: 1 }

ERROR/TOAST PATTERNS:
- Position: fixed top-4 right-4 z-[110]
- Background: bg-red-900/80 border border-red-500/50
- Text: text-red-200
- Animation: slide in from right, auto-dismiss after 4s
- Style: gentle, not alarming — warm red, not pure #ff0000

HEADER PATTERNS:
- Font: font-heading (Rajdhani)
- Style: text-lg font-bold tracking-widest text-[var(--color-accent)]
- Glow: drop-shadow-[var(--glow-accent)]
- Uppercase: always
- Status indicator: w-2 h-2 rounded-full bg-emerald-500 shadow-[var(--glow-success)]
```

### 8.2 Quick Reference Card

| Element | Classes / Pattern |
|---|---|
| **Glass panel** | `bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)] border border-[var(--color-border)] rounded-xl shadow-lg` |
| **Heading** | `font-heading text-lg font-bold tracking-widest uppercase text-[var(--color-accent)] drop-shadow-[var(--glow-accent)]` |
| **Body text** | `font-body text-[var(--color-text-primary)] text-[15px] leading-relaxed` |
| **Muted text** | `text-[var(--color-text-muted)] text-xs` |
| **Primary button** | `bg-[var(--color-accent)] text-black rounded-lg px-4 py-2 hover:bg-white transition-colors` |
| **Ghost button** | `border border-[var(--color-border)] text-[var(--color-text-secondary)] rounded-lg px-4 py-2 hover:border-[var(--color-accent)]` |
| **Text input** | `bg-black/40 border border-[var(--color-border)] text-[var(--color-text-primary)] rounded-md px-4 py-3 focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)] transition-all` |
| **Status dot (online)** | `w-2 h-2 rounded-full bg-emerald-500 shadow-[var(--glow-success)]` |
| **Status dot (offline)** | `w-2 h-2 rounded-full bg-amber-500` |
| **Divider** | `border-t border-[var(--color-border)]` |
| **Scrollable area** | `overflow-y-auto scrollbar-thin scrollbar-thumb-[var(--color-accent)] scrollbar-track-transparent` |

### 8.3 File Naming Conventions

| Type | Convention | Example |
|---|---|---|
| **Component** | PascalCase `.tsx` | `ChatPanel.tsx`, `ModList.tsx` |
| **Hook** | camelCase `use*.ts` | `useTheme.ts`, `useCharacterState.ts` |
| **Service** | camelCase `.ts` | `ttsService.ts`, `audioPlayer.ts` |
| **Type definitions** | camelCase `.ts` in `core/types/` | `mod.ts`, `chat.ts` |
| **Layout configs** | JSON in `layouts/` | `default.json`, `compact.json` |
| **Theme configs** | camelCase `.ts` in `ui/theme/` | `default.ts`, `warmSunset.ts` |

### 8.4 UX Checklist — Before Shipping Any Component

- [ ] Does it feel warm and character-centric?
- [ ] Does it use theme tokens (`var(--*)`) — no hardcoded colors?
- [ ] Does it follow the spacing scale (4px increments)?
- [ ] Does every action give visible feedback (hover, press, focus)?
- [ ] Does it have smooth transitions via framer-motion?
- [ ] Is text contrast sufficient (≥ 4.5:1 body, ≥ 3:1 large)?
- [ ] Are click targets at least 44×44px?
- [ ] Does it degrade gracefully with `prefers-reduced-motion`?
- [ ] Does it include `aria-label` for icon-only buttons?
- [ ] Does it use `pointer-events-auto` when inside the UI overlay grid?
- [ ] Does scroll stay internal to the component (no page scroll)?
- [ ] Does it register in `ComponentRegistry` with a semantic name?

