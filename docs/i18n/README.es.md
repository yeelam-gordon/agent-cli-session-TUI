# Agent CLI Session TUI — Buscar en el historial y retomar conversaciones

<a id="languages"></a>
<details>
<summary>Languages / 语言 / 言語 / اللغات (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Busca conversaciones entre CLI en una búsqueda. Retoma su contexto con el CLI original instalado. Solo en Windows, intenta enfocar su pestaña Windows Terminal coincidente ya en ejecución, sin abrir otra ventana.

[Ver el flujo](#first-workflow) · [Instalar Core](#install-core) · [Proveedores y límites](../../README.md#supported-providers)

La agrupación remota predeterminada puede enviar títulos y rutas automáticamente; elige Core + wordmatch antes de iniciar.

<img src="../agent-session-benefits.png" alt="Concepto: encontrar y continuar una conversación entre historiales de CLI" width="780">

Ilustración conceptual, no una captura real de la aplicación. Las etiquetas en inglés y las filas ficticias son ejemplos, no historiales privados reales.

- Encuentra un tema o proyecto antiguo: busca una frase que recuerdes y verifica proveedor, título y directorio.
- Continúa la conversación correcta: `Enter` en **Resumable** requiere historial conservado, CLI instalado y autenticado, y directorio original accesible.
- Localiza la pestaña en ejecución en Windows: intenta enfocar una pestaña coincidente de Windows Terminal, solo en Windows; no conecta a cualquier proceso. En Linux/macOS, cambia manualmente.

No hace falta haberla iniciado en este TUI.


<a id="install-core"></a>

## Primera búsqueda privada: Core + wordmatch

Necesitas conversaciones guardadas de al menos un proveedor. Para retomarlas, su CLI debe estar instalado, autenticado y disponible mediante el comando configurado; el directorio de trabajo original debe ser accesible. Usa una terminal interactiva y una versión para tu SO y CPU.

El lanzador predeterminado es `wt` en Windows (alternativa: `cmd`), `gnome-terminal` en Linux y `osascript` con Terminal en macOS; adapta la configuración si faltan. Core contempla x64/arm64 en los tres SO; la disponibilidad semántica depende del destino y de los archivos publicados. El enfoque nativo de pestañas requiere Windows Terminal y se limita a Windows.

En [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), descarga el **archivo Core completo** para tu SO y CPU, no el archivo de código fuente. Extrae todo y conserva el ejecutable, `providers/`, `prompts/` y el `config.toml.example` generado. Comprueba `state_dir`, `command`, el lanzador y la opción de privacidad siguiente. El archivo fuente `config.toml.template` contiene marcadores y no se debe simplemente renombrar.

Comprueba el digest del archivo exacto antes de iniciar; la coincidencia no es firma ni garantía de seguridad. Respeta avisos del SO y políticas de tu organización. [SHA-256](../../README.md#verify-a-release-download).

**Aviso antes del primer inicio:** la agrupación `remote`, predeterminada, envía títulos y directorios de trabajo a un servicio alojado y puede ejecutarse automáticamente después de detectar el historial. No hay un contrato público de API para terceros. Para agrupar sin red, sigue la configuración inicial a continuación:

Para una primera instalación de Core, copia el `config.toml.example` generado como `config.toml` junto al ejecutable, sin sobrescribir una configuración existente. En esa copia, busca `# [grouping]` y `# engine = "remote"`: quita el `#` inicial de ambas líneas y cambia el valor a `"wordmatch"`, como abajo. Deja comentadas las demás opciones. Si ya existe una tabla `[grouping]` activa, modifica su `engine` sin añadir otra tabla. Guarda antes de iniciar o explorar el historial.

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` compara palabras: no es IA local. El complemento semántico opcional puede descargar modelos al inicializarse; empieza con Core para no cargarlo. Esta opción no controla las conexiones de los propios CLI.

<a id="first-workflow"></a>

## Primera búsqueda y resultado esperado

Empieza con un fragmento literal del título o proyecto. Core ofrece búsqueda por palabras y aproximada; las paráfrasis semánticas requieren el plugin opcional.

Inicia desde una terminal en la carpeta extraída: `.\agent-session-tui.exe` en PowerShell de Windows o `./agent-session-tui` en Linux/macOS. Espera a que aparezcan las sesiones, pulsa `/` y escribe un proyecto o una frase que recuerdes de tu propio historial. Revisa los detalles con las flechas; confirma proveedor, título y directorio antes de pulsar `Enter`.

Se incluyen seis definiciones de historial y rutas de reanudación configuradas; las versiones de CLI nativos probadas aquí son **desconocidas**. No es una certificación de compatibilidad en vivo. Véase [cobertura y evidencia](../../README.md#supported-providers).

| UI | `Enter` |
|---|---|
| Resumable | Reanudar con CLI instalado y autenticado y directorio original accesible |
| Running / Waiting / Idle | Solo en Windows, intentar enfocar la pestaña en ejecución coincidente de Windows Terminal; en otros sistemas, cambiar manualmente |
| Archived / Stopped | Reanudar con CLI instalado y autenticado y directorio original accesible; [limits](../../README.md#supported-providers) |



Con **Resumable**, el resultado esperado es una terminal donde el CLI carga la conversación elegida para continuar.

`Esc` cancela la búsqueda; `q` / `Ctrl+C` sale sin borrar el historial ni detener otros CLI. La función de terminar sesiones no está implementada.

- Lista vacía: comprueba historial guardado, proveedor habilitado y `state_dir`.
- Fallo al reanudar: revisa instalación del CLI, autenticación y directorio original.
- No inicia: extrae el archivo completo con `providers/`, `prompts/` y ejemplo generado; revisa el lanzador.

Para listas vacías, errores de configuración, fallos de inicio o enfoque, actualización, desinstalación y restauración: [solución de problemas en inglés](../../README.md#troubleshooting-update-and-removal). La [documentación inglesa](../../README.md) es la referencia técnica; consulta [configuración](../../README.md#configuration) y [privacidad de agrupación](../../README.md#what-remote-sends).




Si ya conoces el proveedor y la sesión, basta con el historial del CLI. Este catálogo ayuda cuando trabajas con varios CLI o proyectos y recuerdas el tema, no el ID.

Un gestor de sesiones de terminal para el historial existente de Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI y Kimi, incluidas conversaciones iniciadas por separado. Hay seis proveedores integrados; admitir otros requiere ampliar sus definiciones, no significa compatibilidad con cualquier CLI.

Sigue los pasos de privacidad antes de iniciar; la agrupación remota predeterminada puede ejecutarse automáticamente.





### Windows Terminal

Con **Running/Waiting/Idle**, solo en Windows se intenta enfocar una pestaña coincidente de Windows Terminal: si funciona, se selecciona y la ventana pasa al primer plano. En Linux/macOS debes cambiar manualmente a la terminal en ejecución. Las consultas vagas o con errores no garantizan un resultado.

<details>
<summary>↳ Primera búsqueda privada: Core + wordmatch</summary>

**Ejemplo ficticio, no observado en ejecución ni incluido como sesión:** pulsa `/`, busca `build failure` e imagina una fila Claude Code · título `Build failure` · proyecto `demo-project`. Con `↑`/`↓`, comprueba proveedor, título y directorio original. Si es la conversación **Resumable** correcta, `Enter` la reabre mediante el CLI de Claude instalado y autenticado, con el directorio accesible. No garantiza resultados ni compatibilidad.

</details>



Consulta las conversaciones guardadas en la lista, busca en tu historial y elige una para retomarla. El GIF muestra la interfaz en inglés de la versión grabada, no una verificación de la versión actual ni de la compatibilidad de cada CLI. Traducir la documentación no traduce la interfaz ni las conversaciones guardadas. Configura la privacidad indicada en la instalación antes de iniciar. Se desconocen la versión y la fecha de la grabación. [Instalar Core](#install-core).

<img src="../agentTUI.gif" alt="Demostración del TUI: lista de historial, búsqueda y reanudación" width="700">
