import { useState, useEffect } from 'react';

function Navbar() {
  const [scrolled, setScrolled] = useState(false);

  useEffect(() => {
    const handleScroll = () => setScrolled(window.scrollY > 20);
    window.addEventListener('scroll', handleScroll);
    return () => window.removeEventListener('scroll', handleScroll);
  }, []);

  return (
    <nav className={`fixed top-0 left-0 right-0 z-50 transition-all duration-300 ${scrolled ? 'bg-gray-950/90 backdrop-blur-md shadow-lg' : 'bg-transparent'}`}>
      <div className="max-w-7xl mx-auto px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 bg-gradient-to-br from-orange-500 to-amber-600 rounded-lg flex items-center justify-center">
            <span className="text-white font-bold text-sm">酌</span>
          </div>
          <span className="text-white font-semibold text-xl tracking-tight">Zhuo</span>
        </div>
        <div className="hidden md:flex items-center gap-8">
          <a href="#features" className="text-gray-300 hover:text-orange-400 transition-colors text-sm">Features</a>
          <a href="#code" className="text-gray-300 hover:text-orange-400 transition-colors text-sm">Example</a>
          <a href="#architecture" className="text-gray-300 hover:text-orange-400 transition-colors text-sm">Architecture</a>
          <a href="#getting-started" className="text-gray-300 hover:text-orange-400 transition-colors text-sm">Get Started</a>
        </div>
        <div className="flex items-center gap-4">
          <a href="https://github.com" target="_blank" rel="noopener noreferrer" className="text-gray-400 hover:text-white transition-colors">
            <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24"><path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/></svg>
          </a>
          <a href="#getting-started" className="bg-orange-500 hover:bg-orange-600 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors">
            Get Started
          </a>
        </div>
      </div>
    </nav>
  );
}

function HeroSection() {
  return (
    <section className="relative min-h-screen flex items-center justify-center overflow-hidden">
      {/* Background gradient */}
      <div className="absolute inset-0 bg-gradient-to-br from-gray-950 via-gray-900 to-gray-950" />
      
      {/* Animated grid */}
      <div className="absolute inset-0 opacity-20">
        <div className="absolute inset-0" style={{
          backgroundImage: 'linear-gradient(rgba(251, 146, 60, 0.1) 1px, transparent 1px), linear-gradient(90deg, rgba(251, 146, 60, 0.1) 1px, transparent 1px)',
          backgroundSize: '60px 60px'
        }} />
      </div>

      {/* Glow effects */}
      <div className="absolute top-1/4 left-1/4 w-96 h-96 bg-orange-500/10 rounded-full blur-3xl animate-pulse" />
      <div className="absolute bottom-1/4 right-1/4 w-96 h-96 bg-amber-500/10 rounded-full blur-3xl animate-pulse" style={{ animationDelay: '1s' }} />

      <div className="relative z-10 text-center px-6 max-w-5xl mx-auto">
        <div className="mb-8 inline-flex items-center gap-2 bg-orange-500/10 border border-orange-500/20 rounded-full px-4 py-2">
          <span className="w-2 h-2 bg-orange-400 rounded-full animate-pulse" />
          <span className="text-orange-300 text-sm font-medium">Pure Rust · Zero Dependencies</span>
        </div>
        
        <h1 className="text-6xl md:text-8xl font-bold mb-6">
          <span className="bg-gradient-to-r from-orange-400 via-amber-400 to-orange-500 bg-clip-text text-transparent">酌</span>
          <span className="text-white ml-4 text-5xl md:text-7xl">Zhuo</span>
        </h1>
        
        <p className="text-xl md:text-2xl text-gray-400 mb-4 font-light">
          斟酌古今，精雕细琢
        </p>
        <p className="text-lg text-gray-500 mb-12 max-w-2xl mx-auto">
          A modern, high-performance Rust framework crafted with precision. 
          Built from the ground up with safety, speed, and elegance in mind.
        </p>

        <div className="flex flex-col sm:flex-row items-center justify-center gap-4">
          <a href="#getting-started" className="group bg-gradient-to-r from-orange-500 to-amber-500 hover:from-orange-600 hover:to-amber-600 text-white px-8 py-4 rounded-xl text-lg font-semibold transition-all shadow-lg shadow-orange-500/25 hover:shadow-orange-500/40">
            Get Started
            <span className="inline-block ml-2 group-hover:translate-x-1 transition-transform">→</span>
          </a>
          <a href="#code" className="border border-gray-700 hover:border-orange-500/50 text-gray-300 hover:text-white px-8 py-4 rounded-xl text-lg font-medium transition-all">
            View Example
          </a>
        </div>

        {/* Stats */}
        <div className="mt-20 grid grid-cols-3 gap-8 max-w-lg mx-auto">
          <div>
            <div className="text-3xl font-bold text-white">0</div>
            <div className="text-sm text-gray-500 mt-1">Unsafe Code</div>
          </div>
          <div>
            <div className="text-3xl font-bold text-white">100%</div>
            <div className="text-sm text-gray-500 mt-1">Rust</div>
          </div>
          <div>
            <div className="text-3xl font-bold text-white">∞</div>
            <div className="text-sm text-gray-500 mt-1">Possibilities</div>
          </div>
        </div>
      </div>

      {/* Scroll indicator */}
      <div className="absolute bottom-8 left-1/2 -translate-x-1/2 animate-bounce">
        <svg className="w-6 h-6 text-gray-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 14l-7 7m0 0l-7-7m7 7V3" />
        </svg>
      </div>
    </section>
  );
}

function FeaturesSection() {
  const features = [
    {
      icon: '⚡',
      title: 'Blazing Fast',
      description: 'Zero-cost abstractions and compile-time optimizations ensure maximum performance with minimal runtime overhead.'
    },
    {
      icon: '🛡️',
      title: 'Memory Safe',
      description: 'Leverage Rust\'s ownership model and type system to eliminate entire classes of bugs at compile time.'
    },
    {
      icon: '🧩',
      title: 'Modular Design',
      description: 'Composable architecture with clean abstractions. Use only what you need, nothing more.'
    },
    {
      icon: '🔧',
      title: 'Developer Experience',
      description: 'Intuitive APIs with excellent error messages. From prototype to production with confidence.'
    },
    {
      icon: '🌐',
      title: 'Cross Platform',
      description: 'Build once, run anywhere. Full support for Linux, macOS, Windows, and WebAssembly targets.'
    },
    {
      icon: '📦',
      title: 'Zero Dependencies',
      description: 'Pure Rust implementation with no external dependencies. Minimal attack surface, maximum reliability.'
    }
  ];

  return (
    <section id="features" className="py-32 bg-gray-950 relative">
      <div className="absolute inset-0 bg-gradient-to-b from-gray-950 via-gray-900/50 to-gray-950" />
      <div className="relative max-w-7xl mx-auto px-6">
        <div className="text-center mb-20">
          <h2 className="text-4xl md:text-5xl font-bold text-white mb-6">
            Why <span className="text-orange-400">Zhuo</span>?
          </h2>
          <p className="text-gray-400 text-lg max-w-2xl mx-auto">
            Built with the philosophy of "斟酌" — carefully considering every design decision to deliver the best possible experience.
          </p>
        </div>

        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-6">
          {features.map((feature, index) => (
            <div key={index} className="group p-8 rounded-2xl bg-gray-900/50 border border-gray-800 hover:border-orange-500/30 transition-all duration-300 hover:shadow-lg hover:shadow-orange-500/5">
              <div className="text-4xl mb-4">{feature.icon}</div>
              <h3 className="text-xl font-semibold text-white mb-3 group-hover:text-orange-400 transition-colors">
                {feature.title}
              </h3>
              <p className="text-gray-400 leading-relaxed">
                {feature.description}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function CodeSection() {
  const codeExample = `use zhuo::prelude::*;

#[zhuo::main]
async fn main() -> Result<()> {
    // Initialize the framework
    let app = Zhuo::new()
        .with_name("my-app")
        .with_config(Config::default());

    // Define your services
    app.service(|ctx: &Context| {
        ctx.respond("Hello from Zhuo!")
    });

    // Start the application
    app.run().await?;
    
    Ok(())
}`;

  return (
    <section id="code" className="py-32 bg-gray-950 relative">
      <div className="max-w-7xl mx-auto px-6">
        <div className="grid lg:grid-cols-2 gap-16 items-center">
          <div>
            <h2 className="text-4xl md:text-5xl font-bold text-white mb-6">
              Elegant &<br />
              <span className="text-orange-400">Expressive</span>
            </h2>
            <p className="text-gray-400 text-lg mb-8 leading-relaxed">
              Zhuo provides a clean, idiomatic Rust API that feels natural to use. 
              With powerful macros and async support, you can build complex applications 
              with minimal boilerplate.
            </p>
            <div className="space-y-4">
              <div className="flex items-center gap-3">
                <div className="w-6 h-6 rounded-full bg-orange-500/20 flex items-center justify-center">
                  <span className="text-orange-400 text-xs">✓</span>
                </div>
                <span className="text-gray-300">Async/await native support</span>
              </div>
              <div className="flex items-center gap-3">
                <div className="w-6 h-6 rounded-full bg-orange-500/20 flex items-center justify-center">
                  <span className="text-orange-400 text-xs">✓</span>
                </div>
                <span className="text-gray-300">Derive macros for common patterns</span>
              </div>
              <div className="flex items-center gap-3">
                <div className="w-6 h-6 rounded-full bg-orange-500/20 flex items-center justify-center">
                  <span className="text-orange-400 text-xs">✓</span>
                </div>
                <span className="text-gray-300">Type-safe configuration</span>
              </div>
              <div className="flex items-center gap-3">
                <div className="w-6 h-6 rounded-full bg-orange-500/20 flex items-center justify-center">
                  <span className="text-orange-400 text-xs">✓</span>
                </div>
                <span className="text-gray-300">Comprehensive error handling</span>
              </div>
            </div>
          </div>

          <div className="relative">
            <div className="absolute -inset-1 bg-gradient-to-r from-orange-500/20 to-amber-500/20 rounded-2xl blur-xl" />
            <div className="relative bg-gray-900 rounded-2xl border border-gray-800 overflow-hidden">
              <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-800">
                <div className="w-3 h-3 rounded-full bg-red-500/80" />
                <div className="w-3 h-3 rounded-full bg-yellow-500/80" />
                <div className="w-3 h-3 rounded-full bg-green-500/80" />
                <span className="ml-2 text-gray-500 text-xs font-mono">main.rs</span>
              </div>
              <pre className="p-6 overflow-x-auto text-sm leading-relaxed">
                <code className="text-gray-300 font-mono">
                  {codeExample.split('\n').map((line, i) => (
                    <div key={i} className="flex">
                      <span className="text-gray-600 select-none w-8 text-right mr-4">{i + 1}</span>
                      <span dangerouslySetInnerHTML={{ __html: highlightRust(line) }} />
                    </div>
                  ))}
                </code>
              </pre>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function highlightRust(line: string): string {
  // Simple syntax highlighting
  let result = line
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
  
  // Keywords
  result = result.replace(/\b(use|fn|async|await|let|struct|impl|pub|mod|crate|self|super|where|trait|enum|match|if|else|for|in|while|loop|return|break|continue|type|const|static|mut|ref|move|dyn|as|unsafe|extern)\b/g, 
    '<span class="text-purple-400">$1</span>');
  
  // Types
  result = result.replace(/\b(Result|Ok|Err|String|Vec|Option|Some|None|Box|Arc|Rc|Config|Context|Zhuo)\b/g, 
    '<span class="text-cyan-400">$1</span>');
  
  // Strings
  result = result.replace(/"([^"]*)"/g, '<span class="text-green-400">"$1"</span>');
  
  // Attributes
  result = result.replace(/(#\[[^\]]*\])/g, '<span class="text-yellow-400">$1</span>');
  
  // Comments
  result = result.replace(/(\/\/.*)$/g, '<span class="text-gray-500 italic">$1</span>');
  
  return result;
}

function ArchitectureSection() {
  return (
    <section id="architecture" className="py-32 bg-gray-950 relative">
      <div className="absolute inset-0 bg-gradient-to-b from-gray-950 via-gray-900/30 to-gray-950" />
      <div className="relative max-w-7xl mx-auto px-6">
        <div className="text-center mb-20">
          <h2 className="text-4xl md:text-5xl font-bold text-white mb-6">
            Architecture
          </h2>
          <p className="text-gray-400 text-lg max-w-2xl mx-auto">
            A layered architecture designed for maximum flexibility and performance.
          </p>
        </div>

        <div className="max-w-4xl mx-auto">
          {/* Architecture diagram */}
          <div className="space-y-4">
            <div className="bg-gradient-to-r from-orange-500/10 to-amber-500/10 border border-orange-500/20 rounded-xl p-6 text-center">
              <div className="text-orange-400 font-mono text-sm mb-2">Application Layer</div>
              <div className="text-white font-semibold">Your Business Logic</div>
            </div>
            <div className="flex justify-center">
              <svg className="w-6 h-8 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 14l-7 7m0 0l-7-7" />
              </svg>
            </div>
            <div className="bg-gradient-to-r from-purple-500/10 to-pink-500/10 border border-purple-500/20 rounded-xl p-6 text-center">
              <div className="text-purple-400 font-mono text-sm mb-2">Framework Layer</div>
              <div className="text-white font-semibold">Zhuo Core · Router · Middleware · Services</div>
            </div>
            <div className="flex justify-center">
              <svg className="w-6 h-8 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 14l-7 7m0 0l-7-7" />
              </svg>
            </div>
            <div className="bg-gradient-to-r from-cyan-500/10 to-blue-500/10 border border-cyan-500/20 rounded-xl p-6 text-center">
              <div className="text-cyan-400 font-mono text-sm mb-2">Runtime Layer</div>
              <div className="text-white font-semibold">Async Runtime · I/O · Memory Management</div>
            </div>
            <div className="flex justify-center">
              <svg className="w-6 h-8 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 14l-7 7m0 0l-7-7" />
              </svg>
            </div>
            <div className="bg-gradient-to-r from-green-500/10 to-emerald-500/10 border border-green-500/20 rounded-xl p-6 text-center">
              <div className="text-green-400 font-mono text-sm mb-2">Platform Layer</div>
              <div className="text-white font-semibold">OS Abstraction · System Calls · Hardware</div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function GettingStartedSection() {
  return (
    <section id="getting-started" className="py-32 bg-gray-950 relative">
      <div className="max-w-7xl mx-auto px-6">
        <div className="text-center mb-20">
          <h2 className="text-4xl md:text-5xl font-bold text-white mb-6">
            Get <span className="text-orange-400">Started</span>
          </h2>
          <p className="text-gray-400 text-lg max-w-2xl mx-auto">
            Up and running in minutes. Zhuo is designed to get you productive quickly.
          </p>
        </div>

        <div className="grid md:grid-cols-3 gap-8 max-w-5xl mx-auto">
          <div className="relative">
            <div className="absolute -top-4 -left-4 w-10 h-10 bg-orange-500 rounded-full flex items-center justify-center text-white font-bold text-lg shadow-lg shadow-orange-500/30">
              1
            </div>
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-8 pt-10">
              <h3 className="text-white font-semibold text-lg mb-3">Install</h3>
              <div className="bg-gray-950 rounded-lg p-4 font-mono text-sm">
                <span className="text-green-400">$</span>
                <span className="text-gray-300 ml-2">cargo add zhuo</span>
              </div>
            </div>
          </div>

          <div className="relative">
            <div className="absolute -top-4 -left-4 w-10 h-10 bg-orange-500 rounded-full flex items-center justify-center text-white font-bold text-lg shadow-lg shadow-orange-500/30">
              2
            </div>
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-8 pt-10">
              <h3 className="text-white font-semibold text-lg mb-3">Configure</h3>
              <div className="bg-gray-950 rounded-lg p-4 font-mono text-sm">
                <span className="text-gray-500">// Zhuo.toml</span><br/>
                <span className="text-orange-400">name</span>
                <span className="text-gray-300"> = </span>
                <span className="text-green-400">"my-app"</span>
              </div>
            </div>
          </div>

          <div className="relative">
            <div className="absolute -top-4 -left-4 w-10 h-10 bg-orange-500 rounded-full flex items-center justify-center text-white font-bold text-lg shadow-lg shadow-orange-500/30">
              3
            </div>
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-8 pt-10">
              <h3 className="text-white font-semibold text-lg mb-3">Run</h3>
              <div className="bg-gray-950 rounded-lg p-4 font-mono text-sm">
                <span className="text-green-400">$</span>
                <span className="text-gray-300 ml-2">cargo run</span>
              </div>
            </div>
          </div>
        </div>

        {/* Terminal demo */}
        <div className="mt-16 max-w-3xl mx-auto">
          <div className="bg-gray-900 rounded-2xl border border-gray-800 overflow-hidden shadow-2xl">
            <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-800 bg-gray-900/50">
              <div className="w-3 h-3 rounded-full bg-red-500/80" />
              <div className="w-3 h-3 rounded-full bg-yellow-500/80" />
              <div className="w-3 h-3 rounded-full bg-green-500/80" />
              <span className="ml-2 text-gray-500 text-xs font-mono">terminal</span>
            </div>
            <div className="p-6 font-mono text-sm space-y-2">
              <div>
                <span className="text-green-400">$</span>
                <span className="text-gray-300 ml-2">cargo new my-app && cd my-app</span>
              </div>
              <div>
                <span className="text-green-400">$</span>
                <span className="text-gray-300 ml-2">cargo add zhuo</span>
              </div>
              <div className="text-gray-500">
                &nbsp;&nbsp;&nbsp;&nbsp;Adding zhuo v0.1.0 to dependencies
              </div>
              <div>
                <span className="text-green-400">$</span>
                <span className="text-gray-300 ml-2">cargo run</span>
              </div>
              <div className="text-gray-500">
                &nbsp;&nbsp;&nbsp;Compiling zhuo v0.1.0
              </div>
              <div className="text-gray-500">
                &nbsp;&nbsp;&nbsp;Compiling my-app v0.1.0
              </div>
              <div className="text-green-400">
                &nbsp;&nbsp;&nbsp;✓ Zhuo started successfully on http://localhost:3000
              </div>
              <div className="text-orange-400 animate-pulse">
                &nbsp;&nbsp;&nbsp;▊
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function Footer() {
  return (
    <footer className="bg-gray-950 border-t border-gray-800 py-16">
      <div className="max-w-7xl mx-auto px-6">
        <div className="grid md:grid-cols-4 gap-12">
          <div className="md:col-span-2">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-8 h-8 bg-gradient-to-br from-orange-500 to-amber-600 rounded-lg flex items-center justify-center">
                <span className="text-white font-bold text-sm">酌</span>
              </div>
              <span className="text-white font-semibold text-xl">Zhuo</span>
            </div>
            <p className="text-gray-400 max-w-md leading-relaxed">
              A pure Rust framework built with care and precision. 
              斟酌古今，精雕细琢 — Deliberating on the past and present, crafting with meticulous precision.
            </p>
          </div>
          <div>
            <h4 className="text-white font-semibold mb-4">Resources</h4>
            <ul className="space-y-2">
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">Documentation</a></li>
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">API Reference</a></li>
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">Examples</a></li>
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">Blog</a></li>
            </ul>
          </div>
          <div>
            <h4 className="text-white font-semibold mb-4">Community</h4>
            <ul className="space-y-2">
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">GitHub</a></li>
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">Discord</a></li>
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">Discussions</a></li>
              <li><a href="#" className="text-gray-400 hover:text-orange-400 transition-colors text-sm">Contributing</a></li>
            </ul>
          </div>
        </div>
        <div className="mt-12 pt-8 border-t border-gray-800 flex flex-col md:flex-row items-center justify-between gap-4">
          <p className="text-gray-500 text-sm">
            © 2026 Zhuo Project. Built with 🦀 and ❤️
          </p>
          <p className="text-gray-600 text-sm">
            Licensed under MIT OR Apache-2.0
          </p>
        </div>
      </div>
    </footer>
  );
}

export default function App() {
  return (
    <div className="bg-gray-950 min-h-screen">
      <Navbar />
      <HeroSection />
      <FeaturesSection />
      <CodeSection />
      <ArchitectureSection />
      <GettingStartedSection />
      <Footer />
    </div>
  );
}
