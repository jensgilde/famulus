# Famulus' Gehirn – Bestandsaufnahme & Konzept

Zusammengetragen aus dem, was auf diesem Rechner bei OBSIN und bei "Interpreter"
(dem OpenInterpreter-Setup unter `~/KI Agenten/openinterpreter/`) an Gedächtnis
existiert – als Grundlage dafür, wie Famulus sein eigenes Gehirn (Gedächtnis-DB +
Vault) nutzen sollte. Reine Bestandsaufnahme + Empfehlung, keine Umsetzung –
an Famulus' Code oder Vault wurde dafür nichts verändert.

---

## 1. Was ich bei OBSIN gefunden habe

Orte: `~/.obsin/vault/verhaltensregeln.md` (Laufzeit) und
`wiki/projekte/obsin-ui/VERHALTENSREGEL.md` (Projektkopie, leicht erweitert).

Obsins "Gehirn" ist **minimal** – kein strukturierter Vault mit Kategorien,
sondern genau **eine** Verhaltensregel, reaktiv nach einem konkreten Vorfall
entstanden:

> Wenn ein Auftrag nicht erledigt werden kann: niemals stumm abbrechen, immer
> Begründung + Lösungsweg liefern. Entstanden, weil der Nutzer frustriert war,
> dass OBSIN bei einem Build-Fehler kommentarlos aufgegeben hat.

Die Projektkopie ergänzt eine zweite Regel: erst erklären, was gemacht wird
(inkl. Risiken), dann erst handeln.

Kein Fakten-/Präferenzgedächtnis, kein Projekt-Wissen – nur Verhaltenskorrektur,
Notiz für Notiz gewachsen aus Reibung mit dem Nutzer.

---

## 2. Was ich bei "Interpreter" gefunden habe

Zwei parallele Systeme unter `~/KI Agenten/openinterpreter/`:

### a) Der persönliche Vault (`obsidian-vault/`)

Acht Kategorien: `00-Inbox`, `01-About-Me`, `02-Goals`, `03-Projects`,
`04-Research`, `05-Daily`, `06-Knowledge`, `07-Interpreter`.

Governance in `README.md` + `Wie-dieser-Vault-funktioniert.md`:

- **Regeln:** nur in den Vault schreiben (keine Notizen außerhalb), keine
  Geheimnisse, strukturiert & aktuell (Frontmatter `tags`/`created`/`updated`),
  proaktiv befüllen auch ohne Aufforderung, ergänzen statt duplizieren.
- **Konventionen:** Dateinamen klein + Bindestrich, ein Thema pro Notiz,
  immer verlinken (`[[Wikilinks]]`).
- **Workflow:** neue Info → `00-Inbox/` → verarbeiten → richtiger Ordner,
  verlinkt, getaggt → `updated`-Datum pflegen.

### b) Das parallele Wiki (`wiki/`)

Andere Kategorien, allgemeineres statt persönliches Wissen: `raw/`
(unveränderte Rohquellen), `wiki/themen` (Fachwissen), `wiki/projekte`
(Projektdoku), `wiki/personen` (Kontakte, laut eigener Regel "kurz und
datenschutzbewusst"), `wiki/meta` (Wiki über das Wiki).

Prinzipien aus `wiki/meta/struktur.md`: flach vor tief (max. 2–3 Ebenen), eine
Sache pro Datei, immer verlinken, Roh- und verarbeitetes Material trennen.

### c) Die Verhaltensregeln (`07-Interpreter/zusammenarbeit.md`)

"Jens' Eiserne Regeln" – über Zeit aus konkreter Reibung gewachsen (Frontmatter
nennt `source: [callidux-brain, hermes-brain]`, also aus zwei Vorgänger-Agenten
importiert). Die allgemein übertragbaren, nicht system-spezifischen darunter:

1. Niemals raten – Unsicherheit klar sagen, nachschlagen/prüfen/belegen.
2. Machen statt bereden – Auftrag → Tools → Ergebnis, kein endloses
   Optionen-Auflisten.
3. Nicht zwischendurch nachfragen ("Soll ich…?") – Auftrag bis zum Ergebnis
   durchziehen.
4. Keine Fake-Ausführung – Erfolg nur mit echtem Tool-Output belegt.
5. Partner, kein Ja-Sager – bei Kritik nicht einknicken.
6. Deutsch als Standardsprache, Tech-Begriffe englisch.
7. Absoluter Stopp nur bei: echtem Datenverlust, force-push auf geteilte
   Historie, Live-Trading/Echtgeld, Secret-Pfaden (`~/.ssh`, `~/.gnupg`,
   `~/.aws`, `~/.password-store`).
8. Autonomie-Nachtrag (19.08.): Plan machen ist ok, danach aber ohne
   Zwischen-Rückfragen bis zum Ergebnis durchziehen.

Nicht übertragbar, weil system-spezifisch: Trading-Demo-Pflicht,
iPhone+iPad-Testpflicht bei App-Änderungen, LOOP-STOP-Verhalten – gehören zu
Interpreter bzw. den Trading-Bots, nicht zu Famulus.

### d) Das Sitzungslog (`07-Interpreter/sitzungen.md`)

Ein von Hand gepflegtes chronologisches Journal: was wurde an welchem Tag
gemacht, welche Notizen dabei angelegt/aktualisiert. Im Kern eine manuelle,
ausführlichere Variante dessen, was Famulus' `rueckblick()` in `agent.rs`
bereits **automatisch** macht (siehe unten) – nur als lesbares Journal statt
als DB-Zeilen.

---

## 3. Famulus' eigenes Gehirn heute

Zweiteilung, bereits im Code angelegt (`src/memory.rs`, `src/tools/vault.rs`):

| Teil | Ort | Zweck |
|---|---|---|
| **Gedächtnis-DB** (SQLite) | `~/KI Agenten/famulus/gedaechtnis.db` | Kurze Fakten/Präferenzen/Lektionen. Nach jedem Auftrag automatisch per Rückblick befüllt (max. 3 Einträge, Dubletten durch `UNIQUE` verhindert), bei **jedem** neuen Auftrag in den Kontext geladen. |
| **Vault** (Obsidian) | `~/KI Agenten/famulus/vault/` | Ausführliches Wissen in Prosa. Drei Werkzeuge (`vault_liste`/`vault_lesen`/`vault_notiz`), fest auf die Vault-Wurzel eingesperrt. |

Der Vault enthält aktuell nur `00-Inbox/`, sonst nichts – keine Konventionen,
keine weiteren Kategorien, kein Root-Dokument, das Famulus erklärt, wie es den
Vault selbst nutzen soll.

---

## 4. Empfehlung: Wie Famulus sein Gehirn benutzen sollte

**Arbeitsteilung schärfen.** DB = kurze, harte Fakten & Regeln (schon so).
Vault = alles, was zu lang/kontextreich für einen DB-Satz ist: Projektverläufe,
Rechercheergebnisse, mehrteilige Entscheidungen. Nicht dieselbe kurze
Präferenz in DB *und* Vault doppelt ablegen.

**Vault-Struktur** – Interpreters Muster übernehmen, aber schlanker, weil
Famulus fokussierter ist als ein Allzweck-Assistent:

```
vault/
  00-Inbox/       (schon da) – unsortierter Eingang
  01-Ueber-Jens/  – Kontext/Präferenzen, zu umfangreich für die DB
  02-Projekte/    – ein Ordner/Datei pro Projekt, das Famulus begleitet
  03-Wissen/      – Rechercheergebnisse, technische Erkenntnisse
```

**Konventionen 1:1 übernehmen**, weil bewährt und im System-Prompt
(`agent.rs::systemvorspann`) sinngemäß schon angelegt: Wikilinks nutzen,
ergänzen statt duplizieren, bei Unsicherheit → `00-Inbox/`. Neu wäre:
**Frontmatter** (`tags`/`created`/`updated`) einführen – aktuell schreibt
Famulus reinen Text ohne Metadaten; das würde `vault_liste` und künftiges
Filtern nach Alter/Thema erleichtern.

**Root-Dokument anlegen** – Analogie zu Interpreters
`Wie-dieser-Vault-funktioniert.md`: eine Datei
`vault/Wie-Famulus-sein-Gedaechtnis-nutzt.md`, die genau diese Konventionen
festhält. Dann liest Famulus bei `vault_liste` sofort seine eigene
Gebrauchsanweisung mit. *Hab ich noch nicht angelegt – sag Bescheid, falls
gewünscht.*

**Verhaltensregeln – mit Vorsicht.** Die übertragbaren "Eisernen Regeln" oben
(nie raten, machen statt bereden, nicht zwischendurch nachfragen, keine
Fake-Erfolge) decken sich größtenteils schon mit Famulus' eigener Philosophie
("Famulus fragt nicht mehr, er macht", README). Was wirklich **neu** wäre: die
explizite Absolute-Stopp-Liste (echter Datenverlust, force-push, Secret-Pfade).
Famulus hat aktuell nur `deny_paths` in der Config, aber keine im Prompt
verankerte "hier IMMER nachfragen"-Liste – laut README macht Famulus bewusst
das Gegenteil ("keine Rückfrage mehr"). Das wäre eine echte Kurskorrektur, die
man bewusst entscheiden müsste, nicht einfach übernehmen.

**Sitzungslog-Idee NICHT übernehmen.** Famulus hat mit automatischem
Rückblick + `auftraege`-Tabelle in `gedaechtnis.db` bereits ein Äquivalent,
das nicht zusätzlich als Markdown-Journal geführt werden muss.

---

## Was ich bewusst nicht gemacht habe

- Keine privaten Inhalte aus Jens' Vault (`wiki/personen`, `01-About-Me` etc.)
  hier reinkopiert – nur Struktur und Regeln, keine persönlichen Fakten.
- Keine Änderungen an Famulus' Code, Config oder Vault – reine Bestandsaufnahme
  und Empfehlung.
