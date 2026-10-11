# Agent CLI Session TUI — Buscar en el historial y retomar conversaciones

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

¿Recuerdas el trabajo, pero no en qué CLI estaba? Busca conversaciones guardadas de Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI y Kimi. Si ya conoces el proveedor y la sesión, puede bastar con el historial del propio CLI.

<img src="../agent-session-benefits.png" alt="Concepto: buscar en historiales CLI, continuar el contexto original e intentar seleccionar la pestaña de Windows en ejecución correspondiente" width="780">

Ilustración conceptual, no captura real de la aplicación. Etiquetas en inglés y filas ficticias.

- **Encuentra el trabajo anterior:** una búsqueda en varios historiales conservados, incluso de conversaciones iniciadas fuera de este TUI.
- **Continúa la conversación original:** recupera su contexto guardado mediante el CLI original instalado.
- **Localiza su pestaña activa en Windows:** intenta seleccionar una pestaña coincidente de Windows Terminal; en Linux/macOS cambia manualmente.

<a id="install-core"></a>

## Primera búsqueda privada: Core + wordmatch

Necesitas historial guardado, el CLI instalado y autenticado, accesible mediante el `command` configurado, y el directorio original disponible. Usa un terminal interactivo y una versión para tu SO/CPU. Lanzadores predeterminados: Windows `wt` (alternativa `cmd`), Linux `gnome-terminal`, macOS `osascript` con Terminal. Si faltan, adapta la [configuración](../usage.md#configuration).

**Antes de iniciar:** la agrupación remota predeterminada puede enviar automáticamente **títulos y directorios de trabajo** a un servicio alojado tras detectar el historial. No hay contrato público de API para terceros. Core + `wordmatch` evita esa agrupación remota, no las conexiones de los propios CLI. El plugin semántico opcional puede descargar modelos al inicializarse; empieza con Core sin ese plugin.

1. Descarga de [Archivos Core por sistema y CPU](../usage.md#core-archive-filenames) el **archivo Core completo** para tu SO/CPU, no el del código fuente. Extrae todo y conserva juntos el ejecutable, `providers/`, `prompts/` y el `config.toml.example` generado. [Comprueba el digest del archivo exacto](../usage.md#verify-a-release-download); coincidir no significa firma ni garantía de seguridad. Respeta avisos del SO y políticas de tu organización.
2. En una instalación nueva, copia el ejemplo generado como `config.toml` junto al ejecutable. **No sobrescribas ajustes existentes ni renombres el `config.toml.template` del código fuente.** En la copia, quita el `#` inicial de **ambas líneas**, `# [grouping]` y `# engine = "remote"`, y cambia el valor:

```toml
[grouping]
engine = "wordmatch"
```

Deja comentadas las demás opciones. Si ya existe una tabla `[grouping]` activa, cambia su `engine` sin duplicarla. Revisa `state_dir` y `command`; **guarda antes de iniciar**. `wordmatch` compara palabras compartidas; no es IA local.

3. Inicia desde un terminal en la carpeta extraída:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## Primera búsqueda y resultado esperado

4. Espera la lista, pulsa `/` y escribe un fragmento literal del título o proyecto de tu historial. Con `↑`/`↓`, verifica proveedor, título y directorio; elige **Resumable** y pulsa `Enter`. **Resultado esperado:** el CLI original abre la conversación seleccionada para continuar. La búsqueda puede omitir resultados; las versiones nativas de CLI probadas son desconocidas. [Cobertura y evidencia](../usage.md#supported-providers).

<a id="windows-terminal"></a>

En **Running/Waiting/Idle**, en Windows el TUI solo intenta enfocar una pestaña coincidente; no se adjunta al proceso. Repetir pronto `Enter` en la misma sesión activa puede **forzar su reanudación** en vez de cambiar de pestaña. Verifica primero. [Ruta de Windows](../usage.md#secondary-workflow-find-a-running-windows-tab).

**¿Sin resultados?** Comprueba historial conservado, proveedor habilitado y `state_dir`; si no inicia, revisa autenticación, directorio original y lanzador. [Solución de problemas](../usage.md#troubleshooting-update-and-removal).

**Salir:** `Esc` cancela la búsqueda; después, `q` cierra el TUI, o usa `Ctrl+C` en la lista o durante la búsqueda sin borrar historiales ni detener otros CLI. Terminar sesiones no está implementado. [Actualizar, restaurar y quitar](../usage.md#troubleshooting-update-and-removal).

La [guía de uso y evidencia en inglés](../usage.md) conserva cifras completas, resultados adversos y límites. La versión y fecha de la [grabación](../usage.md#recorded-demo) se desconocen; no demuestra compatibilidad actual.
