// Cena 3D surreal do overlay (JRV-51) — porta de docs/design/openjarvisbr-surreal.html
// para um módulo reutilizável. Mantém o núcleo holográfico deformável, as
// geometrias orbitais, o campo de partículas e o bloom, mas sem a UI de
// demonstração (dock, drag, som, mic próprio): quem dirige o estado e o
// nível de áudio é o overlay, via engine://state e engine://level.
import * as THREE from "three";

export type SurrealState = "standby" | "listening" | "speaking" | "muted" | "error";

export interface SurrealScene {
    mount(el: HTMLElement): void;
    unmount(): void;
    setState(state: SurrealState): void;
    setLevel(mic: number, model: number): void;
    setIntensity(value: number): void;
}

interface StateProfile {
    activity: number;
    speed: number;
    heat: number;
    coral: number;
}

const STATE_PROFILES: Record<SurrealState, StateProfile> = {
    standby: { activity: 0.18, speed: 0.16, heat: 0.12, coral: 0 },
    listening: { activity: 0.54, speed: 0.37, heat: 0.03, coral: 0 },
    speaking: { activity: 0.95, speed: 0.7, heat: 0.95, coral: 0 },
    muted: { activity: 0.15, speed: 0.08, heat: 0.05, coral: 1 },
    error: { activity: 0.32, speed: 0.14, heat: 0.6, coral: 1 },
};

const TAU = Math.PI * 2;
const CORAL = new THREE.Color("#ff6363");
const MAX_PIXEL_RATIO = 2;

// Ruído/deformação compartilhados pela superfície sólida e pela nuvem de
// pontos que a acompanha — mesma fórmula do protótipo original.
const FIELD_GLSL = `
    uniform float uTime, uEnergy, uLevel, uHeat, uPulse;
    float hash(vec3 p) { p=fract(p*.3183099+vec3(.17,.31,.53)); p*=17.; return fract(p.x*p.y*p.z*(p.x+p.y+p.z)); }
    float noise(vec3 p) {
        vec3 i=floor(p),f=fract(p); f=f*f*(3.-2.*f);
        return mix(mix(mix(hash(i),hash(i+vec3(1,0,0)),f.x),mix(hash(i+vec3(0,1,0)),hash(i+vec3(1,1,0)),f.x),f.y),
                   mix(mix(hash(i+vec3(0,0,1)),hash(i+vec3(1,0,1)),f.x),mix(hash(i+vec3(0,1,1)),hash(i+vec3(1,1,1)),f.x),f.y),f.z);
    }
    float field(vec3 p) { return noise(p)*.64+noise(p*2.03+3.7)*.25+noise(p*4.07)*.11; }
    vec3 deform(vec3 p) {
        vec3 n=normalize(p); float t=uTime*.42;
        float f=field(n*2.7+vec3(t*.5,-t*.35,t*.26));
        float wave=sin(n.y*5.+t*1.7+sin(n.x*5.-t))*sin(n.z*4.-t*.7);
        float distortion=(f-.5)*(.32+uEnergy*.48)+wave*(.035+uLevel*.075);
        p*=1.+distortion+uPulse*.07;
        float angle=sin(n.y*2.+t*.7)*(.13+uEnergy*.14);
        p.xz=mat2(cos(angle),-sin(angle),sin(angle),cos(angle))*p.xz;
        return p;
    }
`;

export function createSurrealScene(): SurrealScene {
    const clamp01 = (value: number): number => Math.min(1, Math.max(0, value));

    let seed = 73621;
    const random = (): number => {
        seed = (Math.imul(seed, 1664525) + 1013904223) | 0;
        return (seed >>> 0) / 4294967296;
    };

    const U = {
        uTime: { value: 0 },
        uEnergy: { value: 0.25 },
        uLevel: { value: 0 },
        uHeat: { value: 0.12 },
        uPulse: { value: 0 },
        uDpr: { value: 1 },
    };

    const renderer = new THREE.WebGLRenderer({
        antialias: false,
        alpha: true,
        premultipliedAlpha: false,
        powerPreference: "high-performance",
    });
    renderer.setClearColor(0x000000, 0);
    renderer.toneMapping = THREE.NoToneMapping;
    renderer.outputColorSpace = THREE.LinearSRGBColorSpace;
    renderer.domElement.style.width = "100%";
    renderer.domElement.style.height = "100%";
    renderer.domElement.style.display = "block";

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(39, 1, 0.1, 100);
    camera.position.set(0, 0, 10.8);
    const world = new THREE.Group();
    scene.add(world);

    const colors = {
        cyan: new THREE.Color("#5fe9ff"),
        blue: new THREE.Color("#267dff"),
        orange: new THREE.Color("#ff7537"),
        violet: new THREE.Color("#a76dff"),
    };

    // Núcleo holográfico deformável.
    const orbGeometry = new THREE.SphereGeometry(1.22, 96, 72);
    const orbMaterial = new THREE.ShaderMaterial({
        uniforms: U,
        vertexShader:
            FIELD_GLSL +
            `varying vec3 vP,vWorld,vNormal;
            void main(){vP=position;vec3 p=deform(position);vec4 w=modelMatrix*vec4(p,1.);vWorld=w.xyz;vNormal=normalize(mat3(modelMatrix)*normal);gl_Position=projectionMatrix*viewMatrix*w;}`,
        fragmentShader:
            FIELD_GLSL +
            `varying vec3 vP,vWorld,vNormal;
            void main(){
                vec3 normal=normalize(cross(dFdx(vWorld),dFdy(vWorld))); if(!gl_FrontFacing) normal=-normal;
                vec3 viewDir=normalize(cameraPosition-vWorld);float rim=pow(1.-max(0.,dot(normal,viewDir)),2.4);
                float f=field(vP*3.1+vec3(uTime*.12,-uTime*.17,uTime*.09));
                float flow=f*26.+vP.y*3.8-uTime*.33;
                float veins=pow(.5+.5*sin(flow*3.4),18.);
                float fine=pow(.5+.5*sin(vP.y*144.+f*23.-uTime*1.6),12.);
                float islands=smoothstep(.25,.8,f);
                float hot=smoothstep(.0,.85,-vP.x*.55-vP.y*.38+f*.22+uHeat*.5);
                vec3 cold=mix(vec3(.025,.1,.62),vec3(.075,.75,1.0),smoothstep(.22,.8,f+vP.y*.2));
                cold=mix(cold,vec3(.40,.09,.82),smoothstep(.2,1.,vP.x*.7-vP.y*.4)*.72);
                vec3 pigment=mix(cold,vec3(1.0,.22,.045),hot*.9);
                float light=.035+islands*.12+rim*1.05+veins*(.35+uEnergy*.34)+fine*.10;
                vec3 color=pigment*light;
                color+=vec3(.4,.85,1.)*pow(rim,3.)*.44;
                color+=pigment*veins*pow(f,2.)*.7;
                float scan=pow(.5+.5*sin(vP.y*43.-uTime*2.2),32.);
                color+=cold*scan*.08*uEnergy;
                gl_FragColor=vec4(color,1.);
            }`,
    });
    const orb = new THREE.Mesh(orbGeometry, orbMaterial);
    world.add(orb);

    const skinCount = 2600;
    const skinPositions = new Float32Array(skinCount * 3);
    for (let i = 0; i < skinCount; i++) {
        const y = 1 - (2 * (i + 0.5)) / skinCount;
        const r = Math.sqrt(1 - y * y);
        const a = i * 2.39996323;
        skinPositions.set([Math.cos(a) * r * 1.235, y * 1.235, Math.sin(a) * r * 1.235], i * 3);
    }
    const skinGeometry = new THREE.BufferGeometry();
    skinGeometry.setAttribute("position", new THREE.BufferAttribute(skinPositions, 3));
    const skinMaterial = new THREE.ShaderMaterial({
        uniforms: U,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        vertexShader:
            FIELD_GLSL +
            `uniform float uDpr; varying float vLight; varying vec3 vColor;
            void main(){vec3 p=deform(position); vec4 mv=modelViewMatrix*vec4(p,1.);vLight=.25+.75*pow(.5+.5*sin(position.y*30.+position.x*17.+uTime),4.);vColor=mix(vec3(.18,.6,1.),vec3(1.,.32,.08),smoothstep(.1,.9,-position.x+uHeat*.6));gl_PointSize=clamp((8.+uLevel*6.)*uDpr/-mv.z,1.,3.8);gl_Position=projectionMatrix*mv;}`,
        fragmentShader: `varying float vLight; varying vec3 vColor;void main(){float r=length(gl_PointCoord-.5);if(r>.5)discard;gl_FragColor=vec4(vColor,(1.-smoothstep(.05,.5,r))*vLight*.65);}`,
    });
    orb.add(new THREE.Points(skinGeometry, skinMaterial));

    // Máquinas orbitais holográficas: arcos interrompidos, marcações radiais.
    const orbits: { group: THREE.Group; index: number; z: number; x: number }[] = [];
    function orbit(radius: number, color: THREE.Color, rotation: [number, number, number], index: number): void {
        const group = new THREE.Group();
        group.rotation.set(...rotation);
        world.add(group);
        const arcMaterial = new THREE.MeshBasicMaterial({
            color: color.clone().multiplyScalar(1.9),
            side: THREE.DoubleSide,
            transparent: true,
            opacity: 0.64,
            depthWrite: false,
            blending: THREE.AdditiveBlending,
        });
        for (let i = 0; i < 5; i++) {
            const start = (i * TAU) / 5 + 0.08;
            const length = (0.64 + random() * 0.35) * (TAU / 5);
            const arc = new THREE.Mesh(new THREE.RingGeometry(radius, radius + 0.007, 80, 1, start, length), arcMaterial);
            group.add(arc);
            if (i % 2 === 0) {
                const band = new THREE.Mesh(
                    new THREE.RingGeometry(radius + 0.035, radius + 0.065, 60, 1, start + 0.05, length * 0.62),
                    new THREE.MeshBasicMaterial({
                        color,
                        side: THREE.DoubleSide,
                        transparent: true,
                        opacity: 0.1,
                        depthWrite: false,
                        blending: THREE.AdditiveBlending,
                    }),
                );
                group.add(band);
            }
        }
        const positions: number[] = [];
        for (let i = 0; i < 128; i++) {
            if (i % 13 > 9) continue;
            const a = (i / 128) * TAU;
            const r = radius + 0.12;
            const l = i % 8 === 0 ? 0.065 : 0.022;
            positions.push(Math.cos(a) * r, Math.sin(a) * r, 0, Math.cos(a) * (r + l), Math.sin(a) * (r + l), 0);
        }
        const geo = new THREE.BufferGeometry();
        geo.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
        group.add(
            new THREE.LineSegments(
                geo,
                new THREE.LineBasicMaterial({ color, transparent: true, opacity: 0.42, blending: THREE.AdditiveBlending, depthWrite: false }),
            ),
        );
        orbits.push({ group, index, z: rotation[2], x: rotation[0] });
    }
    orbit(1.88, colors.orange, [1.09, 0.35, -0.35], 0);
    orbit(2.16, colors.cyan, [0.37, -0.61, 0.55], 1);
    orbit(2.58, colors.blue, [1.42, -0.25, -0.22], 2);

    // Duas curvas impossíveis contínuas atravessam os instrumentos fragmentados.
    const threads: THREE.Line[] = [];
    for (let j = 0; j < 2; j++) {
        const a = new Float32Array(360 * 3);
        for (let i = 0; i < 360; i++) {
            const t = (i / 359) * TAU;
            const r = 1.54 + 0.13 * Math.cos(t * 3 + j);
            a.set([r * Math.cos(t), (1.48 + 0.12 * Math.sin(t * 4)) * Math.sin(t), 0.27 * Math.sin(t * 3 + j)], i * 3);
        }
        const geo = new THREE.BufferGeometry();
        geo.setAttribute("position", new THREE.BufferAttribute(a, 3));
        const line = new THREE.Line(
            geo,
            new THREE.LineBasicMaterial({ color: j ? colors.violet : colors.cyan, transparent: true, opacity: 0.26, blending: THREE.AdditiveBlending, depthWrite: false }),
        );
        line.rotation.set(0.3 + j * 0.6, j * 0.8, -0.4);
        world.add(line);
        threads.push(line);
    }

    const crystalGeometry = new THREE.OctahedronGeometry(1);
    const polyGeometry = new THREE.IcosahedronGeometry(1, 0);
    const crystals: { group: THREE.Group; angle: number; radius: number; height: number; speed: number }[] = [];
    for (let i = 0; i < 14; i++) {
        const group = new THREE.Group();
        const shape = i % 3 === 0 ? polyGeometry : crystalGeometry;
        const color = i % 4 === 0 ? colors.orange : i % 3 === 0 ? colors.violet : colors.cyan;
        const surface = new THREE.Mesh(shape, new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.075, depthWrite: false, blending: THREE.AdditiveBlending }));
        const edge = new THREE.LineSegments(
            new THREE.EdgesGeometry(shape),
            new THREE.LineBasicMaterial({ color: color.clone().multiplyScalar(1.25), transparent: true, opacity: 0.7, blending: THREE.AdditiveBlending, depthWrite: false }),
        );
        group.add(surface, edge);
        const size = 0.065 + random() * 0.1;
        group.scale.set(size, size * (i % 3 === 0 ? 1 : 1.85), size);
        world.add(group);
        crystals.push({
            group,
            angle: (i / 14) * TAU + 0.2,
            radius: 2.55 + random() * 0.9,
            height: (random() - 0.5) * 0.8,
            speed: (0.035 + random() * 0.045) * (i % 2 ? 1 : -1),
        });
    }

    // Todo o movimento das partículas é computado na GPU; sem upload de posição por quadro.
    const particleCount = 5200;
    const particlePositions = new Float32Array(particleCount * 3);
    const seeds = new Float32Array(particleCount * 4);
    for (let i = 0; i < particleCount; i++) {
        const r = 1.55 + Math.pow(random(), 1.8) * 3.5;
        seeds.set([random() * TAU, r, random(), random()], i * 4);
        particlePositions[i * 3] = 1;
    }
    const particleGeometry = new THREE.BufferGeometry();
    particleGeometry.setAttribute("position", new THREE.BufferAttribute(particlePositions, 3));
    particleGeometry.setAttribute("aSeed", new THREE.BufferAttribute(seeds, 4));
    const particleMaterial = new THREE.ShaderMaterial({
        uniforms: U,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        vertexShader: `uniform float uTime,uEnergy,uLevel,uHeat,uPulse,uDpr;attribute vec4 aSeed;varying vec3 vColor;varying float vAlpha;
            void main(){float r=aSeed.y+uPulse*(.2+aSeed.z)*.35;float a=aSeed.x+uTime*(.025+aSeed.z*.05)*(aSeed.z>.5?1.:-1.);float wave=sin(a*3.+uTime*.3+aSeed.w*9.);float y=(aSeed.z-.5)*(.22+uEnergy*.38)+wave*.12; if(aSeed.w>.92)y+=(aSeed.z-.5)*4.;r+=sin(a*5.+uTime)*.025*uEnergy;vec3 p=vec3(cos(a)*r,y,sin(a)*r*.84);p.xy=mat2(.96,-.28,.28,.96)*p.xy;p.yz=mat2(.80,-.60,.60,.80)*p.yz;vec4 mv=modelViewMatrix*vec4(p,1.);float hot=smoothstep(.30,.8,aSeed.w+uHeat*.16);vColor=mix(vec3(.10,.58,1.0),vec3(1.,.30,.065),hot);vColor=mix(vColor,vec3(.5,.16,1.),step(.80,aSeed.z)*.65);vAlpha=(.16+aSeed.z*.55)*(1.-smoothstep(2.4,5.1,r))*(.58+uLevel*.8);gl_PointSize=clamp((6.+aSeed.w*13.+uLevel*9.)*uDpr/-mv.z,.7,4.5);gl_Position=projectionMatrix*mv;}`,
        fragmentShader: `varying vec3 vColor;varying float vAlpha;void main(){float d=length(gl_PointCoord-.5);if(d>.5)discard;float glow=exp(-d*d*22.);gl_FragColor=vec4(vColor,glow*vAlpha);}`,
    });
    const particles = new THREE.Points(particleGeometry, particleMaterial);
    particles.frustumCulled = false;
    world.add(particles);

    const starPositions = new Float32Array(500 * 3);
    for (let i = 0; i < 500; i++) {
        starPositions.set([(random() - 0.5) * 28, (random() - 0.5) * 18, -2 - random() * 12], i * 3);
    }
    const starGeometry = new THREE.BufferGeometry();
    starGeometry.setAttribute("position", new THREE.BufferAttribute(starPositions, 3));
    const stars = new THREE.Points(
        starGeometry,
        new THREE.PointsMaterial({ color: "#6a94b3", size: 0.012, transparent: true, opacity: 0.47, depthWrite: false, blending: THREE.AdditiveBlending }),
    );
    scene.add(stars);

    const haloTexture = (() => {
        const c = document.createElement("canvas");
        c.width = c.height = 128;
        const ctx = c.getContext("2d") as CanvasRenderingContext2D;
        const g = ctx.createRadialGradient(64, 64, 0, 64, 64, 64);
        g.addColorStop(0, "rgba(255,255,255,.28)");
        g.addColorStop(0.3, "rgba(255,255,255,.11)");
        g.addColorStop(0.65, "rgba(255,255,255,.025)");
        g.addColorStop(1, "rgba(255,255,255,0)");
        ctx.fillStyle = g;
        ctx.fillRect(0, 0, 128, 128);
        return new THREE.CanvasTexture(c);
    })();
    const halos: THREE.Sprite[] = [];
    for (const [x, y, color, size] of [
        [-0.8, -0.5, colors.orange, 5],
        [0.6, 0.6, colors.cyan, 5.2],
        [0.8, -0.4, colors.violet, 4.2],
    ] as [number, number, THREE.Color, number][]) {
        const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: haloTexture, color, transparent: true, opacity: 0.3, blending: THREE.AdditiveBlending, depthWrite: false }));
        sprite.position.set(x, y, -1.6);
        sprite.scale.set(size, size, 1);
        world.add(sprite);
        halos.push(sprite);
    }

    // Bloom separável em um quarto de resolução, seguido de um composite —
    // sem pós-processamento pesado, sem shadow maps.
    const targetType = renderer.extensions.has("EXT_color_buffer_float") ? THREE.HalfFloatType : THREE.UnsignedByteType;
    const target = new THREE.WebGLRenderTarget(1, 1, { type: targetType, depthBuffer: true });
    const blurA = new THREE.WebGLRenderTarget(1, 1, { type: targetType, depthBuffer: false });
    const blurB = blurA.clone();
    const postScene = new THREE.Scene();
    const postCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
    const postVertex = "varying vec2 vUv;void main(){vUv=uv;gl_Position=vec4(position.xy,0.,1.);}";
    const blurMaterial = new THREE.ShaderMaterial({
        depthTest: false,
        depthWrite: false,
        uniforms: { tSource: { value: null }, uStep: { value: new THREE.Vector2() }, uExtract: { value: 0 } },
        vertexShader: postVertex,
        fragmentShader: `uniform sampler2D tSource;uniform vec2 uStep;uniform float uExtract;varying vec2 vUv;vec3 sampleAt(vec2 p){vec3 c=texture2D(tSource,p).rgb;return mix(c,max(c-vec3(.18),vec3(0.)),uExtract);}void main(){vec3 c=sampleAt(vUv)*.227027;c+=(sampleAt(vUv+uStep*1.384615)+sampleAt(vUv-uStep*1.384615))*.316216;c+=(sampleAt(vUv+uStep*3.230769)+sampleAt(vUv-uStep*3.230769))*.070270;gl_FragColor=vec4(c,1.);}`,
    });
    const uMuted = { value: 0 };
    const uCoral = { value: CORAL };
    const composite = new THREE.ShaderMaterial({
        depthTest: false,
        depthWrite: false,
        uniforms: {
            tScene: { value: target.texture },
            tBloom: { value: blurB.texture },
            uTime: U.uTime,
            uEnergy: U.uEnergy,
            uPulse: U.uPulse,
            uResolution: { value: new THREE.Vector2() },
            uMuted,
            uCoral,
        },
        vertexShader: postVertex,
        fragmentShader: `uniform sampler2D tScene,tBloom;uniform float uTime,uEnergy,uPulse,uMuted;uniform vec3 uCoral;uniform vec2 uResolution;varying vec2 vUv;
            float rnd(vec2 p){return fract(sin(dot(p,vec2(12.9898,78.233)))*43758.5453);}
            void main(){vec2 uv=vUv;float gate=step(.965,rnd(vec2(floor(uTime*.8),7.)))*uEnergy;float band=step(abs(uv.y-fract(uTime*.071)),.014);uv.x+=(gate*band+uPulse*.05)*sin(uv.y*180.)*.008;float shift=.00022+uEnergy*.00035+uPulse*.0012;vec4 base=texture2D(tScene,uv);vec3 c=vec3(texture2D(tScene,uv+vec2(shift,0)).r,base.g,texture2D(tScene,uv-vec2(shift,0)).b);vec3 bloom=texture2D(tBloom,uv).rgb;c+=bloom*(.95+uEnergy*.40);float scan=1.-(.022+.018*uEnergy)*(.5+.5*sin(vUv.y*uResolution.y*2.2));c*=scan;c=(c*(2.51*c+.03))/(c*(2.43*c+.59)+.14);c=pow(clamp(c,0.,1.),vec3(1./2.2));float gray=dot(c,vec3(.299,.587,.114));c=mix(c,vec3(gray),uMuted*.85);c=mix(c,uCoral*(.4+gray*.9),uMuted*.6);gl_FragColor=vec4(max(c,0.),max(base.a,clamp(max(bloom.r,max(bloom.g,bloom.b))*2.5,0.,1.)));}`,
    });
    const quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), blurMaterial);
    postScene.add(quad);

    let state: SurrealState = "standby";
    let intensity = 0.65;
    let micLevel = 0;
    let modelLevel = 0;
    let level = 0;
    let energy = 0.25;
    let heat = 0.12;
    let pulse = 0.25;
    let coralMix = 0;
    let time = 0;
    let motion = 0;
    let lastTime = performance.now();
    let width = 1;
    let height = 1;
    let mounted = false;
    let visible = false;
    let rafId = 0;

    const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");

    let fpsFrames = 0;
    let fpsTime = performance.now();

    function targetLevel(): number {
        if (state === "listening") return micLevel;
        if (state === "speaking") return modelLevel;
        return 0;
    }

    function resize(): void {
        const dpr = Math.min(window.devicePixelRatio || 1, MAX_PIXEL_RATIO);
        renderer.setPixelRatio(dpr);
        renderer.setSize(width, height, false);
        target.setSize(Math.max(1, Math.round(width * dpr)), Math.max(1, Math.round(height * dpr)));
        const blurW = Math.max(1, Math.round(width * dpr * 0.32));
        const blurH = Math.max(1, Math.round(height * dpr * 0.32));
        blurA.setSize(blurW, blurH);
        blurB.setSize(blurW, blurH);
        camera.aspect = width / height;
        camera.updateProjectionMatrix();
        U.uDpr.value = dpr;
        composite.uniforms.uResolution.value.set(width * dpr, height * dpr);
    }

    function render(): void {
        renderer.setRenderTarget(target);
        renderer.clear();
        renderer.render(scene, camera);
        quad.material = blurMaterial;
        blurMaterial.uniforms.tSource.value = target.texture;
        blurMaterial.uniforms.uStep.value.set(1.6 / blurA.width, 0);
        blurMaterial.uniforms.uExtract.value = 1;
        renderer.setRenderTarget(blurA);
        renderer.render(postScene, postCamera);
        blurMaterial.uniforms.tSource.value = blurA.texture;
        blurMaterial.uniforms.uStep.value.set(0, 1.6 / blurA.height);
        blurMaterial.uniforms.uExtract.value = 0;
        renderer.setRenderTarget(blurB);
        renderer.render(postScene, postCamera);
        quad.material = composite;
        renderer.setRenderTarget(null);
        renderer.render(postScene, postCamera);
    }

    function update(dt: number): void {
        const profile = STATE_PROFILES[state];
        const blend = 1 - Math.exp(-dt * 5);
        const paused = reducedMotion.matches;
        if (!paused) {
            level += (targetLevel() - level) * (1 - Math.exp(-dt * 12));
            energy += (profile.activity * (0.2 + intensity * 0.9) + level * intensity * 0.52 - energy) * blend;
            heat += (profile.heat - heat) * blend;
            time += dt;
            motion += dt * profile.speed * (0.4 + intensity);
            pulse *= Math.exp(-dt * 2.8);
        } else {
            energy = profile.activity * (0.2 + intensity * 0.9);
            heat = profile.heat;
            pulse = 0;
        }
        coralMix += (profile.coral - coralMix) * blend;

        world.rotation.set(Math.sin(time * 0.05) * 0.05, motion * 0.12, 0);
        orb.rotation.set(Math.sin(time * 0.1) * 0.14, motion * 0.16, Math.sin(time * 0.14) * 0.1);
        const scale = 1 + Math.sin(time * 0.8) * 0.018 + level * intensity * 0.045;
        orb.scale.setScalar(scale);
        orbits.forEach(({ group, index, z, x }) => {
            group.rotation.z = z + motion * (index % 2 ? -0.19 : 0.17);
            group.rotation.x = x + Math.sin(time * 0.17 + index) * 0.055;
            group.scale.setScalar(1 + level * 0.025 + pulse * 0.018);
        });
        threads.forEach((thread, i) => {
            thread.rotation.z = motion * (i ? -0.12 : 0.14);
            thread.rotation.y = motion * 0.05 + i * 0.8;
        });
        crystals.forEach(({ group, angle, radius, height: h, speed }) => {
            const a = angle + motion * speed;
            group.position.set(Math.cos(a) * radius, Math.sin(a) * radius * 0.57 + h, Math.sin(a * 2 + 0.8) * 0.85);
            group.rotation.set(time * 0.1 + angle, time * 0.13 + angle, time * 0.055);
        });
        particles.rotation.y = motion * 0.025;
        stars.rotation.z = time * 0.0008;
        halos.forEach((halo, i) => {
            (halo.material as THREE.SpriteMaterial).opacity = 0.24 + energy * 0.13 + Math.sin(time * 0.5 + i) * 0.03;
        });

        U.uTime.value = time;
        U.uEnergy.value = energy;
        U.uLevel.value = level;
        U.uHeat.value = heat;
        U.uPulse.value = pulse;
        uMuted.value = coralMix;
    }

    function tick(now: number): void {
        rafId = requestAnimationFrame(tick);
        const dt = Math.min(Math.max((now - lastTime) / 1000, 0.001), 0.05);
        lastTime = now;
        if (!visible) return;
        update(dt);
        render();

        if (import.meta.env.DEV) {
            fpsFrames++;
            if (now - fpsTime >= 2000) {
                // eslint-disable-next-line no-console
                console.debug(`[surreal] ${Math.round((fpsFrames * 1000) / (now - fpsTime))} FPS`);
                fpsFrames = 0;
                fpsTime = now;
            }
        }
    }

    let resizeObserver: ResizeObserver | null = null;
    let visibilityObserver: MutationObserver | null = null;
    let visibilityRoot: Element | null = null;

    function updateVisibility(): void {
        const root = visibilityRoot;
        visible = mounted && !document.hidden && (root ? root.classList.contains("visible") : true);
        if (visible) lastTime = performance.now();
    }

    return {
        mount(el: HTMLElement): void {
            if (mounted) return;
            mounted = true;
            el.appendChild(renderer.domElement);
            width = Math.max(1, el.clientWidth);
            height = Math.max(1, el.clientHeight);
            resize();

            resizeObserver = new ResizeObserver((entries) => {
                const entry = entries[0];
                if (!entry) return;
                width = Math.max(1, Math.round(entry.contentRect.width));
                height = Math.max(1, Math.round(entry.contentRect.height));
                resize();
            });
            resizeObserver.observe(el);

            visibilityRoot = el.closest(".overlay-container");
            visibilityObserver = new MutationObserver(updateVisibility);
            if (visibilityRoot) {
                visibilityObserver.observe(visibilityRoot, { attributes: true, attributeFilter: ["class"] });
            }
            document.addEventListener("visibilitychange", updateVisibility);
            updateVisibility();

            lastTime = performance.now();
            rafId = requestAnimationFrame(tick);
        },

        unmount(): void {
            if (!mounted) return;
            mounted = false;
            visible = false;
            cancelAnimationFrame(rafId);
            resizeObserver?.disconnect();
            visibilityObserver?.disconnect();
            document.removeEventListener("visibilitychange", updateVisibility);
            renderer.domElement.remove();
            renderer.dispose();
            target.dispose();
            blurA.dispose();
            blurB.dispose();
            haloTexture.dispose();
            [orbGeometry, skinGeometry, particleGeometry, starGeometry, crystalGeometry, polyGeometry].forEach((g) => g.dispose());
            [orbMaterial, skinMaterial, particleMaterial, blurMaterial, composite].forEach((m) => m.dispose());
        },

        setState(next: SurrealState): void {
            state = next;
        },

        setLevel(mic: number, model: number): void {
            micLevel = clamp01(mic);
            modelLevel = clamp01(model);
        },

        setIntensity(value: number): void {
            if (!Number.isFinite(value)) return;
            intensity = clamp01(value);
        },
    };
}
