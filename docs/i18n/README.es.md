# Agent CLI Session TUI — Buscar el historial y retomar conversaciones

[English](../../README.md) · [简体中文](README.zh-CN.md) · [日本語](README.ja.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md) · [Deutsch](README.de.md)

Un gestor de sesiones de terminal para el historial existente de Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI y Kimi, incluidas conversaciones iniciadas por separado. Hay seis proveedores integrados; admitir otros requiere ampliar sus definiciones, no significa compatibilidad con cualquier CLI.

## Antes de iniciar: requisitos, privacidad e instalación

Necesitas conversaciones guardadas de al menos un proveedor. Para retomarlas, su CLI debe estar instalado, autenticado y disponible mediante el comando configurado; el directorio de trabajo original debe ser accesible. Usa una terminal interactiva y una versión para tu SO y CPU.

**Aviso antes del primer inicio:** la agrupación `remote`, predeterminada, envía títulos y directorios de trabajo a un servicio alojado y puede ejecutarse automáticamente después de detectar el historial. No hay un contrato público de API para terceros. Para agrupar sin red, edita la tabla existente de `config.toml` antes de iniciar, sin duplicarla:

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` compara palabras: no es IA local. El complemento semántico opcional puede descargar modelos al inicializarse; empieza con Core para no cargarlo. Esta opción no controla las conexiones de los propios CLI.

En [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), descarga el **archivo Core completo** para tu SO y CPU, no el archivo de código fuente. Extrae todo y conserva el ejecutable, `providers/`, `prompts/` y el `config.toml.example` generado. Copia el ejemplo como `config.toml` junto al ejecutable sin sobrescribir una configuración existente. Comprueba `state_dir`, `command`, el lanzador y la opción de privacidad anterior. El archivo fuente `config.toml.template` contiene marcadores y no se debe simplemente renombrar.

## Primera búsqueda y resultado esperado

Inicia desde una terminal en la carpeta extraída: `.\agent-session-tui.exe` en PowerShell de Windows o `./agent-session-tui` en Linux/macOS. Espera a que aparezcan las sesiones, pulsa `/` y escribe un proyecto o una frase que recuerdes de tu propio historial. Revisa los detalles con las flechas; confirma proveedor, título y directorio antes de pulsar `Enter`.

Con **Resumable**, el resultado esperado es una terminal donde el CLI carga la conversación elegida para continuar. Con **Running/Waiting**, solo en Windows se intenta enfocar una pestaña coincidente de Windows Terminal: si funciona, se selecciona y la ventana pasa al primer plano. En Linux/macOS debes cambiar manualmente a la terminal en ejecución. Las consultas vagas o con errores no garantizan un resultado.

El lanzador predeterminado es `wt` en Windows (alternativa: `cmd`), `gnome-terminal` en Linux y `osascript` con Terminal en macOS; adapta la configuración si faltan. Core contempla x64/arm64 en los tres SO; la disponibilidad semántica depende del destino y de los archivos publicados. `Esc` cancela la búsqueda; `q` / `Ctrl+C` sale sin borrar el historial ni detener otros CLI. La función de terminar sesiones no está implementada.

Para listas vacías, errores de configuración, fallos de inicio o enfoque, actualización, desinstalación y restauración: [solución de problemas en inglés](../../README.md#troubleshooting-update-and-removal). La [documentación inglesa](../../README.md) es la referencia técnica; consulta [configuración](../../README.md#configuration) y [privacidad de agrupación](../../README.md#what-remote-sends).
