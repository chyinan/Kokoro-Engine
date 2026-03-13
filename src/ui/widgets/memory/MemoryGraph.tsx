import { useRef, useEffect, useState, useMemo, type MouseEvent as ReactMouseEvent } from "react";
import type { MemoryRecord } from "../../../lib/kokoro-bridge";

interface MemoryGraphProps {
    memories: MemoryRecord[];
    onSelectKeyword?: (keyword: string) => void;
}

interface Node {
    id: string; // keyword
    count: number;
    x: number;
    y: number;
    vx: number;
    vy: number;
    radius: number;
}

interface Link {
    source: string;
    target: string;
    strength: number;
}

// Common stopwords to filter out
const STOPWORDS = new Set([
    "the", "be", "to", "of", "and", "a", "in", "that", "have", "i", "it", "for", "not", "on", "with", "he", "as", "you", "do", "at",
    "this", "but", "his", "by", "from", "they", "we", "say", "her", "she", "or", "an", "will", "my", "one", "all", "would", "there",
    "their", "what", "so", "up", "out", "if", "about", "who", "get", "which", "go", "me", "when", "make", "can", "like", "time", "no",
    "just", "know", "take", "people", "into", "year", "your", "good", "some", "could", "them", "see", "other", "than", "then", "now",
    "look", "only", "come", "its", "over", "think", "also", "back", "after", "use", "two", "how", "our", "work", "first", "well", "way",
    "even", "new", "want", "because", "any", "these", "give", "day", "most", "us", "is", "am", "are", "was", "were",
    // 日语助词、助动词、接续词、无实意词
    "の", "は", "が", "を", "に", "で", "と", "も", "な", "だ", "です", "ます", "する", "した", "して", "から", "まで", "より",
    "ない", "ある", "いる", "れる", "られる", "こと", "もの", "ため", "よう", "など", "それ", "これ", "あの", "その",
    "って", "てる", "てた", "てて", "ても", "ては", "たら", "たり", "ので", "のに", "けど", "けれど", "でも", "しか",
    "だけ", "ほど", "くらい", "ぐらい", "さえ", "すら", "こそ", "でも", "しも", "ばかり", "だって", "って", "かな",
    "ね", "よ", "わ", "ぞ", "ぜ", "か", "な", "さ", "や", "ぬ", "ん", "っ", "て", "で", "に", "を", "は", "が",
    "いう", "なる", "くる", "いく", "みる", "おく", "しまう", "あげる", "くれる", "もらう",
    // 活用語尾・接続形
    "しく", "しい", "てい", "てき", "にし", "にな", "にも", "べた", "べき", "べく",
    "とても", "とって", "という", "といえ", "とは", "とも", "とき", "ところ", "ながら",
    "なので", "なのに", "なのか", "なって", "なり", "なら", "ならば", "なく", "なけれ",
    "まして", "まずは", "まだ", "また", "ました", "ません", "ましょ", "まま",
    "れて", "られて", "せて", "させて", "ていた", "ている", "ていく", "てから", "てしまっ",
    "ことが", "ことは", "ことも", "ことで", "ことに", "ことを", "ものの", "ものが", "ものは",
    "ように", "ような", "ようで", "ようが", "ようと", "ほうが", "ほうは", "ほうで",
    "できる", "できた", "できて", "よく", "よくて", "よくも", "てく", "てくれ", "てくる",
    "かって", "かった", "かっては", "かっても",
    // 复合助词
    "にとって", "について", "において", "によって", "に対して", "として", "とともに", "をめぐって",
    "に関して", "に対する", "による", "にわたって", "をはじめ", "をつうじて", "にもかかわらず",
    // 中文停用词
    "的", "了", "是", "在", "我", "有", "和", "就", "不", "人", "都", "一", "个", "上", "也", "很", "到", "说", "要",
    "去", "你", "会", "着", "没有", "看", "好", "自己", "这", "他", "她", "吗", "什么", "那", "里", "吧", "啊", "呢",
    "哦", "嗯", "嘿", "哈", "呀", "哇", "唉", "哎", "喔", "嗨", "哟", "咦", "嗯嗯", "嘿嘿",
    "因为", "所以", "但是", "然后", "如果", "虽然", "已经", "可以", "这个", "那个", "一个", "没有", "知道",
]);

/** Check if a string contains CJK characters */
function hasCJK(text: string): boolean {
    return /[\u3000-\u9fff\uac00-\ud7af\uff00-\uffef]/.test(text);
}

/** Extract keywords from text, supporting both CJK and Latin scripts */
function extractKeywords(text: string): string[] {
    const cleaned = text.toLowerCase().replace(/[.,/#!$%^&*;:{}=\-_`~()[\]"'?!？！。、，…～☺️]/g, "");

    if (hasCJK(cleaned) && typeof Intl !== "undefined" && "Segmenter" in Intl) {
        // Use Intl.Segmenter for CJK-aware word segmentation
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const segmenter = new (Intl as any).Segmenter(undefined, { granularity: "word" });
        const segments = Array.from(segmenter.segment(cleaned)) as Array<{ segment: string; isWordLike: boolean }>;
        return segments
            .filter(s => s.isWordLike)
            .map(s => s.segment.trim())
            .filter(w => w.length >= 2 && !STOPWORDS.has(w));
    }

    // Fallback: whitespace split for Latin scripts
    return cleaned
        .split(/\s+/)
        .filter(w => w.length > 3 && !STOPWORDS.has(w));
}

export function MemoryGraph({ memories, onSelectKeyword }: MemoryGraphProps) {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);
    const [hoveredNode, setHoveredNode] = useState<Node | null>(null);
    const [canvasSize, setCanvasSize] = useState({ width: 600, height: 400 });

    // 响应式 canvas 尺寸
    useEffect(() => {
        const container = containerRef.current;
        if (!container) return;
        const ro = new ResizeObserver(entries => {
            const { width, height } = entries[0].contentRect;
            if (width > 0 && height > 0) {
                setCanvasSize({ width: Math.floor(width), height: Math.floor(height) });
            }
        });
        ro.observe(container);
        return () => ro.disconnect();
    }, []);

    // Process data into graph
    const { nodes, links } = useMemo(() => {
        const keywordCounts: Record<string, number> = {};
        const cooccurrences: Record<string, number> = {}; // "key1|key2" -> count

        memories.forEach(mem => {
            // Keyword extraction with CJK support
            const words = extractKeywords(mem.content);

            const uniqueWords = Array.from(new Set(words));

            uniqueWords.forEach(w => {
                keywordCounts[w] = (keywordCounts[w] || 0) + 1;
            });

            // Links for co-occurrence
            for (let i = 0; i < uniqueWords.length; i++) {
                for (let j = i + 1; j < uniqueWords.length; j++) {
                    const w1 = uniqueWords[i];
                    const w2 = uniqueWords[j];
                    const key = w1 < w2 ? `${w1}|${w2}` : `${w2}|${w1}`;
                    cooccurrences[key] = (cooccurrences[key] || 0) + 1;
                }
            }
        });

        // Filter: only top 50 keywords to prevent clutter
        const topKeywords = Object.entries(keywordCounts)
            .sort(([, a], [, b]) => b - a)
            .slice(0, 50)
            .map(([k]) => k);

        const topSet = new Set(topKeywords);

        const nodes: Node[] = topKeywords.map(k => ({
            id: k,
            count: keywordCounts[k],
            x: Math.random() * 800,
            y: Math.random() * 600,
            vx: 0,
            vy: 0,
            radius: Math.min(Math.max(keywordCounts[k] * 3, 5), 25)
        }));

        const links: Link[] = [];
        Object.entries(cooccurrences).forEach(([key, count]) => {
            const [s, t] = key.split("|");
            if (topSet.has(s) && topSet.has(t) && count > 1) { // Only link if co-occur > 1
                links.push({ source: s, target: t, strength: count });
            }
        });

        return { nodes, links };
    }, [memories]);

    // Force simulation loop
    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;

        let animationId: number;
        const width = canvasSize.width;
        const height = canvasSize.height;
        const center = { x: width / 2, y: height / 2 };

        const tick = () => {
            // Forces
            nodes.forEach(node => {
                // 1. Center gravity
                node.vx += (center.x - node.x) * 0.005;
                node.vy += (center.y - node.y) * 0.005;

                // 2. Repulsion — 考虑节点半径，防止重叠
                nodes.forEach(other => {
                    if (node === other) return;
                    const dx = node.x - other.x;
                    const dy = node.y - other.y;
                    const dist = Math.sqrt(dx * dx + dy * dy) || 0.1;
                    const minDist = node.radius + other.radius + 20;
                    if (dist < minDist) {
                        // 在最小距离内施加强排斥
                        const force = (minDist - dist) * 0.3;
                        node.vx += (dx / dist) * force;
                        node.vy += (dy / dist) * force;
                    } else {
                        // 远距离弱排斥
                        const force = 800 / (dist * dist);
                        node.vx += (dx / dist) * force;
                        node.vy += (dy / dist) * force;
                    }
                });
            });

            // 3. Link attraction
            links.forEach(link => {
                const s = nodes.find(n => n.id === link.source);
                const t = nodes.find(n => n.id === link.target);
                if (!s || !t) return;

                const dx = t.x - s.x;
                const dy = t.y - s.y;
                const dist = Math.sqrt(dx * dx + dy * dy);
                const force = (dist - 150) * 0.008; // Rest length 150

                const fx = (dx / dist) * force;
                const fy = (dy / dist) * force;

                s.vx += fx;
                s.vy += fy;
                t.vx -= fx;
                t.vy -= fy;
            });

            // Apply velocity & damping
            nodes.forEach(node => {
                node.vx *= 0.9;
                node.vy *= 0.9;
                node.x += node.vx;
                node.y += node.vy;

                // Bounds — 底部留出文字标签空间 (radius + 16)
                const labelPad = 16;
                node.x = Math.max(node.radius, Math.min(width - node.radius, node.x));
                node.y = Math.max(node.radius, Math.min(height - node.radius - labelPad, node.y));
            });

            // Render
            ctx.clearRect(0, 0, width, height);

            // Draw links
            ctx.strokeStyle = "rgba(100, 116, 139, 0.2)";
            ctx.lineWidth = 1;
            links.forEach(link => {
                const s = nodes.find(n => n.id === link.source);
                const t = nodes.find(n => n.id === link.target);
                if (s && t) {
                    ctx.beginPath();
                    ctx.moveTo(s.x, s.y);
                    ctx.lineTo(t.x, t.y);
                    ctx.stroke();
                }
            });

            // Draw nodes
            nodes.forEach(node => {
                ctx.beginPath();
                ctx.arc(node.x, node.y, node.radius, 0, Math.PI * 2);
                ctx.fillStyle = hoveredNode === node ? "#3b82f6" : "rgba(59, 130, 246, 0.6)"; // Blue-500
                ctx.fill();

                // Text
                ctx.fillStyle = "#cbd5e1"; // Slate-300
                ctx.font = hoveredNode === node ? "bold 12px Inter" : "10px Inter";
                ctx.textAlign = "center";
                ctx.fillText(node.id, node.x, node.y + node.radius + 12);
            });

            animationId = requestAnimationFrame(tick);
        };

        tick();

        return () => cancelAnimationFrame(animationId);
    }, [nodes, links, hoveredNode, canvasSize]);

    // Interaction handlers
    const handleMouseMove = (e: ReactMouseEvent) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const x = e.clientX - rect.left;
        const y = e.clientY - rect.top;

        const found = nodes.find(n => {
            const dx = n.x - x;
            const dy = n.y - y;
            return dx * dx + dy * dy < (n.radius + 5) * (n.radius + 5);
        });

        setHoveredNode(found || null);
        canvas.style.cursor = found ? "pointer" : "default";
    };

    const handleClick = () => {
        if (hoveredNode && onSelectKeyword) {
            onSelectKeyword(hoveredNode.id);
        }
    };

    return (
        <div ref={containerRef} className="w-full h-[400px] bg-slate-900/50 rounded-lg overflow-hidden border border-slate-800 relative">
            <canvas
                ref={canvasRef}
                width={canvasSize.width}
                height={canvasSize.height}
                className="w-full h-full"
                onMouseMove={handleMouseMove}
                onClick={handleClick}
            />
            {nodes.length === 0 && (
                <div className="absolute inset-0 flex items-center justify-center text-slate-500 text-sm">
                    Not enough data for graph
                </div>
            )}
        </div>
    );
}
