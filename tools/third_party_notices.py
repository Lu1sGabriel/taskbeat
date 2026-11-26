"""Gera THIRD-PARTY-NOTICES.md a partir das dependências do Cargo.

Uso (na raiz do projeto):  python tools/third_party_notices.py
"""
import json
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent

PLATFORMS = {"Windows": "x86_64-pc-windows-msvc", "Mac": "aarch64-apple-darwin", "Linux": "x86_64-unknown-linux-gnu"}


def walk(platform):
    """Dependências (todas, e as que vão no executável) para uma plataforma."""
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", platform], cwd=ROOT))
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]

    # Dependências normais (alcançáveis a partir do app), sem dev-dependencies.
    seen, stack = set(), [root]
    while stack:
        nid = stack.pop()
        for dep in nodes[nid]["deps"]:
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])

    def is_proc_macro(pid):
        return any("proc-macro" in t["kind"] for t in pkgs[pid]["targets"])

    # O que vai no executável: dependências normais, sem entrar em proc-macros
    # nem em dependências de build (essas só rodam na compilação).
    in_app, stack = set(), [root]
    while stack:
        nid = stack.pop()
        for dep in nodes[nid]["deps"]:
            normal = any(k["kind"] is None for k in dep["dep_kinds"])
            if normal and not is_proc_macro(dep["pkg"]) and dep["pkg"] not in in_app:
                in_app.add(dep["pkg"])
                stack.append(dep["pkg"])
    return pkgs, seen, in_app


pkgs, seen, in_app, where = {}, set(), set(), {}
for system, triple in PLATFORMS.items():
    p_, s_, a_ = walk(triple)
    pkgs.update(p_)
    seen |= s_
    in_app |= a_
    for pid in s_:
        where.setdefault(pid, []).append(system)

COPYRIGHT = re.compile(r"^\s*(copyright\b.*)$", re.I | re.M)


def copyright_lines(pkg):
    d = pathlib.Path(pkg["manifest_path"]).parent
    lines = []
    for f in sorted(d.glob("LICENSE*")) + sorted(d.glob("COPYRIGHT*")):
        try:
            text = f.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for m in COPYRIGHT.findall(text):
            m = m.strip()
            # Linhas de modelo do texto da Apache-2.0 não são avisos reais.
            if "copyright notice" in m.lower() or "[yyyy]" in m or "{yyyy}" in m or "owner]" in m or "copyright owner" in m.lower() or "copyright license" in m.lower():
                continue
            if m not in lines:
                lines.append(m)
    return lines


rows = sorted((pkgs[i] for i in seen), key=lambda p: p["name"])
out = []
out.append("# Avisos de terceiros / Third-party notices\n")
out.append("O Taskbeat usa as bibliotecas de código aberto abaixo. Todas têm licenças permissivas")
out.append("(MIT, Apache-2.0, Zlib, BSD-3-Clause e Unicode-3.0), compatíveis com a licença MIT do projeto.")
out.append("Bibliotecas marcadas com *build* só são usadas na compilação e não vão no executável.\n")
out.append("Arquivo gerado por `python tools/third_party_notices.py`.\n")
out.append("| Biblioteca | Versão | Licença | Uso | Sistema | Repositório |")
out.append("|---|---|---|---|---|---|")
for p in rows:
    kind = "app" if p["id"] in in_app else "build"
    repo = p.get("repository") or ""
    ws = where.get(p["id"], [])
    systems = ", ".join(ws[:-1]) + " e " + ws[-1] if len(ws) > 1 else "".join(ws)
    out.append(f"| {p['name']} | {p['version']} | {p.get('license') or '—'} | {kind} | {systems} | {repo} |")
out.append("| mediaremote-adapter | 2ef957c | BSD-3-Clause | app (dentro do Taskbeat.app) | Mac | https://github.com/ungive/mediaremote-adapter |")
out.append("")
out.append("No Linux o app também usa bibliotecas do próprio sistema, que não vão no pacote (são")
out.append("as que já vêm com o desktop, ligadas na hora de abrir): fontconfig (MIT), FreeType (FTL),")
out.append("HarfBuzz (MIT), libdbus (AFL-2.1 ou GPL-2.0) e libpulse (LGPL-2.1).")

out.append("\n## Avisos de copyright\n")
for p in rows:
    lines = copyright_lines(p)
    if lines:
        out.append(f"**{p['name']} {p['version']}**")
        out.append("")
        out.extend(f"- {l}" for l in lines)
        out.append("")

out.append("**mediaremote-adapter** (no pacote do Mac: `MediaRemoteAdapter.framework` e `mediaremote-adapter.pl`)")
out.append("")
out.append("- Copyright (c) 2025 Jonas van den Berg and contributors")
out.append("")
out.append("""```
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```
""")
out.append("## Texto da licença MIT\n")
out.append("""```
Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```
""")
out.append("## Apache-2.0 e Unicode-3.0\n")
out.append("As bibliotecas com licença dupla ou tripla (\"MIT OR Apache-2.0\", \"Zlib OR Apache-2.0 OR MIT\", \"Unlicense OR MIT\") são usadas aqui sob a MIT.")
out.append("Textos completos: [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) e")
out.append("[Unicode-3.0](https://www.unicode.org/license.txt) (usada só por `unicode-ident`, na compilação).\n")

(ROOT / "THIRD-PARTY-NOTICES.md").write_text("\n".join(out), encoding="utf-8")
print(f"{len(rows)} bibliotecas -> THIRD-PARTY-NOTICES.md")
