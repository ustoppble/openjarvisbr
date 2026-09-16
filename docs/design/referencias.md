# Referências visuais — Janela flutuante & Ícone da barra de menu (OpenJarvisBR v2)

**Data:** 2026-09-16  
**Contexto:** Overlay 420×140px, transparente, sempre por cima, estilo Siri. Precisa ler bem em live streaming.

---

## 1. Siri Minimalist Pill

**Inspiração:** macOS 27 (Apple Siri AI Interface)

**Referências:**
- [macOS 27 Beta Siri Voice Pad Interface (MacRumors)](https://www.macrumors.com/2026/07/23/macos-27-beta-hidden-siri-voice-ui/)
- [Apple's Secret Siri Interface in macOS 27](https://www.voiceos.com/blog/apple-siri-ai-wwdc-2026-mac)

**Paleta (hex):**
- Background: `#0a0e27` (nearly black, subtle blue tint)
- Waveform: `#64b5f6` (soft blue) ou `#81c784` (soft green)
- Text: `#ffffff` (bright white)
- Accent: `#ffd54f` (warm yellow)
- Border: `#1a237e` (dark blue)

**Tipografia:**
- Body: -apple-system, BlinkMacSystemFont, SF Pro Display, 11–13px, weight 500

**Onda/Orb:**
- Pill-shaped floating container (rounded rectangle, 20px radius)
- Animated waveform bars (3–5 bars) reacting to mic level in real-time
- Smooth sine-wave interpolation between levels
- Bars scale 0–100% of container height

**Animação de aparecer/sumir:**
- Appear: scale 0.8 → 1 + fade-in (200ms, ease-out)
- Disappear: fade-out (300ms, ease-in) + scale to 0.9

**Prós:**
- Extremamente simples e clara, integra-se ao sistema macOS
- Mostra waveform sem poluição visual
- Legível em stream (contraste alto, shapes simples)
- Menu bar icon monochrome (segue design system Apple)

**Contras:**
- Muito minimalista pode parecer "frio" ou corporativo
- Waveform bars sozinhas são menos atraentes que orbs/circles
- Dificuldade em transmitir "personalidade" do Jarvis

**Recomendação:** Base sólida, excelente pra readability em live, mas considere adicionar um toque de cor quente ou movimento mais orgânico.

---

## 2. Raycast Dark Cockpit

**Inspiração:** Raycast AI Design System

**Referências:**
- [Raycast Design System Analysis](https://getdesign.md/raycast/design-md)
- [Raycast Custom Themes](https://manual.raycast.com/themes)
- [Raycast Pro Features](https://www.raycast.com/pro)

**Paleta (hex):**
- Background: `#0c0c0c` (pure black, almost OLED)
- Canvas: `#1a1a1a` (very dark gray)
- Border: `#363739` (hairline, barely visible)
- Accent: `#ff6363` (warm coral/salmon)
- Text: `#e0e0e0` (off-white)
- Glass edge: `#2a2a2a` (inset highlight)

**Tipografia:**
- Font: Inter, 12px, weight 400–600
- Monospace (SF Mono) para níveis de áudio numéricos

**Onda/Orb:**
- Waveform circles/dots em grid 5×2 (frequências agrupadas)
- Cada dot reage a banda de frequência diferente (baixo → grave, alto → agudo)
- Glow sutil em coral (#ff6363) ao redor dos dots ativos
- Backdrop-filter blur para efeito de glass

**Animação de aparecer/sumir:**
- Appear: backdrop-filter blur(0) → blur(12px) + dots fade-in (150ms)
- Disappear: blur reverso + dots dissolvem (200ms)

**Prós:**
- Design cockpit sofisticado, power-user appeal
- Coral accent é memorável e marca presença
- Glass aesthetic é tendência 2026
- Muito legível em live (preto profundo + branco)

**Contras:**
- Requer cuidado com backdrop-filter em navegadores (não funciona em Firefox)
- Muitos dots podem parecer "ocupados" em janela pequena (420×140)
- Cor coral é ousada demais para assistente corporativo?

**Recomendação:** Excelente se quer visual "power tool" + personalidade. Adapte o grid para 3–4 dots ao invés de 10 para caber na janela.

---

## 3. ChatGPT Floating Overlay

**Inspiração:** OpenAI ChatGPT Desktop App

**Referências:**
- [ChatGPT Display Mode Reference (June 2026)](https://sunpeak.ai/blogs/chatgpt-app-display-mode-reference/)
- [ChatGPT Desktop Productivity](https://www.theaienterprise.io/p/chatgpt-desktop-productivity)
- [OpenAI Plugins UI Guidelines](https://developers.openai.com/plugins/concepts/ui-guidelines)

**Paleta (hex):**
- Background: `#10a37f` (OpenAI teal) ou `#ffffff` (white) com transparência 0.95
- Gradient overlay: `rgba(16, 163, 127, 0.1)` (subtle teal tint)
- Text: `#0d0d0d` (almost black)
- Accent: `#19c37d` (bright teal)
- Subtle shadow: `rgba(0, 0, 0, 0.1)`

**Tipografia:**
- Font: -apple-system, Inter, 12px, weight 500
- Line-height: 1.4

**Onda/Orb:**
- Três pontos pulsantes centralizados (●●●), dispostos horizontalmente
- Cada ponto pulsa com delay escalonado (100ms offset)
- Quando fala, pontos crescem em tamanho (0.4em → 0.6em)
- Quando escuta, pontos respiram suavemente

**Animação de aparecer/sumir:**
- Appear: pontos aparecem com scale 0 + fade-in, respiram imediatamente (keyframes)
- Disappear: pontos param de respirar, desaparecem com fade-out + scale 0 (200ms)

**Prós:**
- Abordagem minimalista da OpenAI é reconhecível
- Teal color é corporativo + modern
- Pontos pulsantes são universalmente compreendidas como "escutando"
- Fácil de adaptar pra tema escuro (teal → cor similar em dark mode)

**Contras:**
- Três pontos sozinhos podem parecer "genérico" demais
- Sem waveform, perde a reação "ao vivo" de frequências
- Teal OpenAI é muito associado a ChatGPT (pode parecer cópia)

**Recomendação:** Solução segura e profissional. Bom pra primeiro MVP, pode ser elevada depois com waveform.

---

## 4. Perplexity Voice Orb

**Inspiração:** Perplexity Pro Voice Mode (iOS)

**Referências:**
- [Perplexity Voice Mode Redesign](https://www.techradar.com/computing/artificial-intelligence/perplexitys-voice-mode-gets-a-futuristic-makeover-on-your-iphone)
- [Voice Mode with Real-Time Answers](https://alternativeto.net/news/2025/2/perplexity-s-lasted-ios-update-introduces-new-redesigned-voice-mode-with-real-time-answers)
- [Engaging with Perplexity's Voice Mode](https://www.firstaimovers.com/p/perplexity-voice-mode)

**Paleta (hex):**
- Background: `#1a1a1a` (very dark gray, OLED-friendly)
- Orb primary: `#6b5ff0` (purple)
- Orb secondary: `#3b82f6` (blue)
- Accent: `#10b981` (emerald green)
- Text: `#ffffff` (white)
- Glow: `rgba(107, 95, 240, 0.4)` (purple glow)

**Tipografia:**
- Font: -apple-system, Inter, 13px, weight 600 (slightly bold)

**Onda/Orb:**
- Esfera de dots (7–9 pontos distribuídos em círculo)
- Quando escuta: dots respiram, giram lentamente ao redor do centro
- Quando fala: dots se "expandem" radialmente, pulam com frequência do áudio
- Centro com círculo menor (ponto de foco)
- Efeito de glow ao redor da esfera inteira

**Animação de aparecer/sumir:**
- Appear: orb cresce do centro (scale 0 → 1), dots fade-in, começam a girar (300ms, ease-out)
- Disappear: orb encolhe para o centro, dots desaparecem (200ms, ease-in)

**Prós:**
- Visual atraente e "futurista", diferencia-se de competitors
- Orb com dots é intuitivo = "sistema pensando"
- Glow é moderno e captura atenção
- Funciona bem em pequenas janelas (dots escalam bem)
- Ótimo pra live streaming (movimento chamativo, cores vibrantes)

**Contras:**
- Mais complexo computacionalmente (muitos dots + animations)
- Pode parecer "distração" se os dots se mexem muito
- Purple + Blue requer cuidado pra não parecer "genérico" (muitos assistentes usam estas cores)

**Recomendação:** Excelente se quer visual "wow" e moderno. Forte candidato pra OpenJarvisBR pois combina movimento com clareza.

---

## 5. JARVIS Holographic HUD

**Inspiração:** Iron Man JARVIS Interface (Cinematic + Developers' Implementations)

**Referências:**
- [JARVIS HUD on GitHub](https://github.com/tejasbankar99/JARVIS)
- [JARVIS AI with Iron Man Interface](https://github.com/mohabtecno-alt/jarvis-ai-ironman)
- [Redesigning the JARVIS UX](https://medium.com/fictional-products-for-fictional-worlds/redesigning-the-jarvis-ux-a-minimalist-approach-to-a-genius-system-208b39113e8d)

**Paleta (hex):**
- Background: `#0a0a0a` (pure black)
- Primary: `#ff6b35` (warm orange/copper — "Arc Reactor")
- Secondary: `#004e89` (deep blue)
- Accent: `#ffd700` (gold)
- Scan line: `rgba(255, 107, 53, 0.3)` (orange glow)
- Text: `#ff6b35` (orange)

**Tipografia:**
- Font: 'Orbitron', 'Space Mono', ou monospace fallback, 11–12px, weight 700
- Letter-spacing: +0.5px (futurista)

**Onda/Orb:**
- Círculo central (Arc Reactor) com pulsação constante
- Anel rotativo ao redor (dashes que giram continuamente)
- Waveform bars irradiando do centro (6–8 barras dispostas radialmente)
- Scan lines horizontais que passam ocasionalmente (efeito CRT)
- Quando escuta: anel gira, bars reagem ao mic
- Quando fala: bars "explodem" para fora, cores alternam orange ↔ gold

**Animação de aparecer/sumir:**
- Appear: Arc Reactor pulsa 1.5x rapidamente (100ms), anel começa a girar, scan lines aparecem (250ms)
- Disappear: anel desacelera, scan lines desaparecem, reactor "encolhe" (300ms)

**Prós:**
- Icônico e memorável (JARVIS é referência em assistentes de voz)
- Orange + gold é quente e atrai atenção em live
- Movimento cinético (anel rotativo) mantém engajamento
- Monospace futurista reforça "AI" vs "humano"
- Muito diferenciado no mercado

**Contras:**
- Muito "ocupado" para janela 420×140 — risco de parecer confuso
- Monospace no corpo de texto é difícil de ler em pequenas fontes
- Scan lines e efeitos CRT podem parecer "nostálgicos" vs "futurista"
- Requer animações complexas (pode ser lento em máquinas fracas)

**Recomendação:** Bom se quer máxima personalidade e diferenciação. Risco: pode ficar cluttered. Teste layout antes de commitar.

---

## 6. Glassmorphism Voice Wave

**Inspiração:** Glassmorphism Trend + Voice Assistant Design Patterns

**Referências:**
- [Glassmorphism and Voice UI (SitePoint)](https://www.sitepoint.com/glassmorphism-and-voice-ui/)
- [Next-Gen AI Voice Assistant UI (Figma)](https://www.figma.com/community/file/1495015707288549907/next-gen-ai-voice-assistant-ui-sleek-smart-intuitive)
- [Voice Assistant Dashboard UI — Dark Mode (Figma)](https://www.figma.com/community/file/1511492451560495975/ai-voice-assistant-dashboard-ui-dark-mode-interface)
- [Building a Voice Reactive Orb in React (Medium)](https://medium.com/@therealmilesjackson/building-a-voice-reactive-orb-in-react-audio-visualization-for-voice-assistants-2bee12797b93)

**Paleta (hex):**
- Background: `rgba(20, 20, 30, 0.8)` (dark navy com transparência)
- Glass: `rgba(50, 50, 70, 0.2)` (frosted glass effect)
- Border: `rgba(100, 150, 200, 0.3)` (subtle light blue border)
- Waveform: `#7c3aed` (vibrant purple) e `#06b6d4` (cyan)
- Text: `#f0f4f8` (light blue-white)
- Glow: `rgba(124, 58, 237, 0.4)` (purple glow)

**Tipografia:**
- Font: -apple-system, Inter, 12px, weight 400
- Letter-spacing: normal

**Onda/Orb:**
- Waveform em estilo "liquid" — curvas suaves (SVG paths) ao invés de barras retas
- Duas ondas sobrepostas: uma purple (#7c3aed), outra cyan (#06b6d4) com opacity 0.7
- Ondas oscillam suavemente, sincronizadas com frequências do áudio
- Backdrop-filter blur(16px) + slight saturate(120%)
- Brilho sutil ao redor (box-shadow com cores vibrantes)

**Animação de aparecer/sumir:**
- Appear: backdrop-filter blur(0) → blur(16px), ondas fade-in com suavidade (200ms)
- Disappear: ondas dissolvem, blur reverte (150ms)

**Prós:**
- Trendy e "2026" — glassmorphism é quente agora
- Duas cores (purple + cyan) criam profundidade visual
- Curvas suaves são mais orgânicas que barras
- Waveform liquid é naturalmente reativa a áudio
- Elegante e sofisticado

**Contras:**
- Backdrop-filter não funciona em todos os navegadores/plataformas
- Curvas suaves perdem detalhe de frequências (menos "technical")
- Purple + cyan pode parecer "gamer" em alguns contextos
- Transparência de fundo pode causar problemas se conteúdo por trás mudar rapidamente

**Recomendação:** Excelente pra um design "premium" e moderno. Bom pra destaque visual em live, mas teste transparência em diferentes cenários.

---

## 7. Minimal Mono Listener

**Inspiração:** Extreme Minimalism + Text-Based UI

**Referências:**
- Prototipagem própria (sem referência específica — abordagem experimental)

**Paleta (hex):**
- Background: `#000000` (pure black)
- Indicator: `#ffffff` (pure white)
- Secondary: `#666666` (gray)
- Status text: `#cccccc` (light gray)

**Tipografia:**
- Font: Menlo, Monaco, 'Courier New' (strict monospace), 10px, weight 400
- All caps mode (opcional)

**Onda/Orb:**
- Texto puro: `[████░░░░]` (barra ASCII)
- Ou símbolo único que muda: `◐ ◑ ◕ ◑ ◐` (fases da lua)
- Ou linha simples: `─ ╱ │ ╲ ─` (rotativa)
- Reação ao nível: largura da barra cresce, símbolo pulsa
- Sem gráficos, sem animações suaves — apenas 4–6 estados discretos

**Animação de aparecer/sumir:**
- Appear: símbolo aparece instantaneamente, barra começa a reagir
- Disappear: desaparece instantaneamente

**Prós:**
- Ultra-simples, sem dependências visuais
- Funciona em qualquer tela (até terminal)
- Extremamente legível em live (contraste máximo, sem ambiguidade)
- Debugável (fácil ver qual estado está ativo)
- Carga mínima de CPU

**Contras:**
- Visualmente "chato" ou "retro"
- Não transmite nenhuma personalidade ou "alma"
- Pode parecer "quebrado" ou "placeholder"
- Não é atraente para usuários não-técnicos

**Recomendação:** Usar apenas se a constraint é "pura legibilidade" ou "fallback para máquinas lentas". Não recomendado pra UX principal.

---

## Recomendação Final

**Direção Escolhida: Perplexity Voice Orb + Raycast Dark Cockpit (Fusão)**

**Justificativa:**

Combinamos o melhor dos dois mundos:
1. **Orb com dots (Perplexity)**: Visual atraente, movimento cinético, fácil de escalar pra janela pequena
2. **Paleta dark cockpit (Raycast)**: `#0c0c0c` background, `#ff6363` ou `#7c3aed` accent, Inter typography

**Paleta Final (hex):**
- Background: `#0c0c0c` (OLED-friendly black)
- Orb primary: `#7c3aed` (vibrant purple — diferencia de ChatGPT/Siri)
- Orb secondary: `#06b6d4` (cyan accent)
- Accent corner/state: `#ff6363` (Raycast coral — mostra quando mudo/erro)
- Text: `#e0e0e0` (off-white, readable)
- Glow: `rgba(124, 58, 237, 0.3)` (subtle purple halo)

**Tipografia:**
- Font: Inter, 12px, weight 500
- Monospace (SF Mono) para estado numérico ou debug

**Onda/Orb:**
- 7 dots dispostos em círculo (Perplexity-style)
- Respiração quando standby, reação de frequência quando escuta
- Glow ao redor da esfera
- Quando fala: dots expandem radialmente + pulsam

**Animação:**
- Appear: scale 0 → 1 + fade-in (250ms, ease-out), dots começam a respirar
- Disappear: fade-out + scale 0.9 (200ms, ease-in)
- Mute toggle: orb desatura (saturate 20%) + border fica coral

**Ícone da Barra de Menu:**
- Versão simplificada: círculo sólido em `#7c3aed` (listening), desaturado quando muted
- Número pequeno (contador) em canto se há mensagens pendentes

**Por que funciona:**
- ✅ Legível em live streaming (contraste alto, cores vibrantes)
- ✅ Diferenciado vs Siri/ChatGPT (purple + cyan é signature)
- ✅ Escalável pra 420×140 (dots escalam bem)
- ✅ Movimento atrai atenção sem ser distrativo
- ✅ Paleta "power tool" + personalidade
- ✅ Pronto pra implementar em SVG/Canvas/CSS animations
- ✅ Funciona bem em dark mode (default)

**Próximos Passos:**
1. Prototipar orb com 7 dots em HTML/CSS + JavaScript (60–90 min)
2. Testar readability em live (gravação OBS)
3. Medir performance (CPU/GPU) em máquinas fracas
4. Feedback do Laschuk antes de commitar no CSS/design

---

## Recursos & Inspiração

**Geral:**
- [LottieFiles — Free Voice Assistant Animations](https://lottiefiles.com/free-animations/voice-assistant)
- [Figma Community — Voice Assistant UI Templates](https://www.figma.com/community)

**Tools pra prototipagem:**
- Figma (design)
- CodePen (CSS animations)
- Observable (waveform visualization prototypes)

**Performance:**
- Use `requestAnimationFrame` pra animations (não setInterval)
- Considerar usar `canvas` ao invés de SVG se CPU for issue
- Lazy-load glow/blur effects em máquinas fracas
