# Revisión de origen e integración

Fecha: 1 de octubre de 2026. Revisión de código y configuración; no se han leído ni utilizado credenciales.

## Fuentes

- Original `Louis-CFM/coucou`: commit `5ae7bd946ab51493b5ddaebdc5f449f269ebb421`.
- Fork `iiZo7al/coucou-chatgpt`: commit `5b4ec94cc32684536e38a9b5bbfabe1bdf4ce43d`.
- Adaptación local derivada del original: el resultado final debe validarse con los builds y pruebas documentados por el proyecto. Esta revisión identifica los problemas de origen y las condiciones que se deben comprobar; no certifica una release.

## Problemas comprobados en el fork de terceros

| Prioridad | Ubicación | Problema y consecuencia |
| --- | --- | --- |
| Alta | `windows/src-tauri/src/settings.rs:26`; `integrations.rs:148` | Siguen usando `crate::claude` después de eliminar `claude.rs` y declarar `mod openai`. El backend Windows no compila sin corregir ambos usos. |
| Alta | `windows/src-tauri/src/secrets.rs` | La lista admite `anthropic-api-key`, pero el cliente y los ajustes usan `openai-api-key`. Guardar la clave falla y el chat no puede autenticarse. |
| Alta | `windows/src/core/state.ts`; `island/hooks.ts`; `island/integrations.ts`; `views/views.ts`; `views/integrations.ts`; `island/island.ts` | El estado crea `integration_codex`, pero handlers y vistas siguen buscando `integration_claude`. Se pierden actualizaciones de sesión, estados, apertura de carpeta, badges y el ticker. También quedan comparaciones `claudeCode` contra un tipo cambiado a `codex`. |
| Alta | `windows/src-tauri/src/hooks.rs` | Identificar un grupo por contener cualquier comando con `coucou-hook` y quitar todo el grupo puede borrar hooks de terceros que lo compartan. La instalación debe filtrar comandos individuales y preservar campos y comandos ajenos. |
| Media | `windows/src-tauri/src/openai.rs` | Los archivos recibidos por IPC se leen desde cualquier ruta. PDF e imágenes no tienen límite previo de tamaño. Hace falta comprobar que cada adjunto pertenece al inbox y rechazar tamaño/tipo no admitidos con un error visible. |
| Media | `windows/src-tauri/src/openai.rs` | Falta `store: false`. No se debe describir el envío al proveedor como exclusivamente local. Las políticas de conservación del proveedor y las de su organización siguen aplicándose. |
| Media | `windows/src-tauri/src/openai.rs` | Cada operación del historial tiene Mutex, pero la transacción completa del turno no se serializa. Dos peticiones IPC simultáneas pueden mezclar historial y rollback. Un resultado sin texto tampoco revierte el mensaje añadido. |
| Media | `NotchBuddy/NotchBuddy.xcodeproj/project.pbxproj:86` | El proyecto comprometido referencia `ClaudeService.swift`, eliminado y sustituido por `OpenAIService.swift`. La CI regenera con XcodeGen; abrir directamente el proyecto guardado no equivale a ese build. |
| Media | `README.md:60`, `:66`, `:80`, `:89`; `docs/index.html`; `scripts/release.sh` | Descargas, clonación y publicación heredadas apuntan al original. Usuarios pueden instalar una versión para Claude o un script puede intentar publicar al repositorio del autor. |
| Media | `.github/workflows/windows.yml` | El fork retiró `PUBLISH: false` y volvió a habilitar publicación. El original la suspendía por una detección de Defender en el instalador sin firma. La afirmación de falso positivo es del autor; esta revisión no analiza esos ejecutables. |

La respuesta de `PermissionRequest` **sí** coincide con la documentación actual de Codex: `hookSpecificOutput.hookEventName` y `decision.behavior` aceptan `allow`/`deny`. No se considera un fallo por conservar esa forma de Claude. La configuración correcta documentada es `~/.codex/hooks.json`; mencionar `~/.codex/settings.json` en el README es un error de documentación. [Hooks oficiales](https://learn.chatgpt.com/docs/hooks).

Instalar el archivo no basta: Codex exige revisar y confiar en la definición exacta mediante `/hooks` antes de ejecutar hooks no gestionados. Una actualización cambia su hash y puede exigir nueva revisión. No debe saltarse esta revisión con `--dangerously-bypass-hook-trust`. `SessionEnd` admite hasta 3 segundos; el fork configura 10 y debe corregirse. [Revisión y límites de hooks](https://learn.chatgpt.com/docs/hooks).

## Problemas heredados que deben tenerse en cuenta

- `pipe.rs` crea una tarea por conexión y lee sin plazo inicial. El límite de bytes debe provocar rechazo efectivo, con plazo de lectura y límite de conexiones. El relay valida el SID del servidor, pero el servidor original no valida el SID del cliente. El nombre de la pipe no autentica por sí solo a quien se conecta.
- El relay lee stdin completo antes de aplicar su plazo de trabajo y trunca strings de comandos a 2.000 bytes. No se debe aprobar desde un resumen incompleto que oculta el resto del comando. Mantener aprobaciones en el cliente oficial es una alternativa segura para esta adaptación.
- `poll_once()` del original evita las condiciones de pausa e integración activada que sí aplica el bucle periódico. Las acciones de refresco deben respetar esos controles.
- El estado de sesiones original es una sola tarjeta global. Los eventos intercalados de dos sesiones se mezclan. Un timer de `Stop` puede borrar un estado de trabajo posterior a los 5,2 segundos. Cancelar o invalidar timers al recibir nueva actividad; no prometer una lista independiente de todas las sesiones sin implementarla.
- Los binarios y datos originales comparten identificador `fr.louisraille.coucou`, directorios `Coucou` y nombre de pipe. Una adaptación instalada en paralelo necesita espacios propios para no sustituir secretos/configuración/hooks ni competir por IPC con el original.
- El DOM muestra las respuestas con `textContent`, lo que evita interpretar HTML del modelo. La CSP limita scripts y no hay plugins de shell/filesystem genéricos en las capabilities. Mantener estas propiedades.

## Dependencias y distribución

Los manifests npm y Cargo del fork de terceros conservan las dependencias del original. El cambio importante de CI es añadir un build de Windows al workflow general y quitar la suspensión de publicación del instalador. Esto no resuelve los errores de módulos encontrados ni valida las releases disponibles.

Se deben conservar lockfiles, usar `npm ci` o la instalación congelada equivalente, y ejecutar un build nativo de Windows, no sólo TypeScript ni un check del cliente portable. No hay firma de Windows configurada en `tauri.conf.json`. Hashes de un artefacto no prueban ausencia de malware, pero permiten vincular la descarga al artefacto revisado.

## Licencias y marca

`LICENSE` autoriza modificar y distribuir el **código** bajo MIT y exige preservar copyright y texto de licencia. `LICENSE-ASSETS.md` reserva al autor nombres Coucou/Mochi, personaje, iconos, sonidos y medios; permite ejecución personal y forks/contribuciones, pero exige permiso escrito para distribuir una app derivada con esos elementos. Antes de publicar binarios propios hay que reemplazar nombre, iconos, personaje y sonidos, o conseguir permiso. Publicar una adaptación no la convierte en producto oficial de OpenAI ni del autor original.

## Condiciones para la adaptación local

1. Inicio de sesión y almacenamiento de sesión gestionados por el CLI oficial; ninguna lectura de `auth.json`, cookies, contraseñas o extracción de tokens.
2. Cliente `codex app-server` por stdio con path absoluto, lanzamiento sin shell, plazos y tamaño de respuesta limitados. No exponer puerto TCP.
3. Modo ChatGPT debe exigir cuenta ChatGPT y no cambiar silenciosamente a facturación API. Modo API debe elegirse explícitamente y mostrar sus costes separados.
4. Chat integrado con permisos mínimos y solicitudes de aprobación desconocidas rechazadas. No activar herramientas con efectos externos sólo porque estén configuradas en el Codex global del usuario.
5. Archivos limitados al inbox y a tipos/tamaños admitidos; errores visibles si no se puede adjuntar. Historial normal, limitado y serializado; reset o cambio de backend no debe resucitar respuestas antiguas.
6. Hook merge preserva config ajena y usa backup, preview y escritura atómica. Monitorización no debe conceder permisos silenciosamente.
7. README anuncia sólo plataformas y formatos realmente implementados; macOS heredado del original continúa siendo Claude hasta que se migre y pruebe.

La documentación oficial presenta app-server para clientes con autenticación, historial y eventos, y mantiene el uso local/open source de su autenticación. Esa autenticación no está permitida para servicios comerciales/hosted; esos productos requieren evaluar Sign in with ChatGPT. [App-server oficial](https://learn.chatgpt.com/docs/app-server).

`store: false` controla el almacenamiento de respuesta por Responses; no elimina todas las políticas de conservación de datos ni los logs de prevención de abuso. [Controles de datos oficiales](https://developers.openai.com/api/docs/guides/your-data).

### Compatibilidad de sandbox del CLI instalado

La documentación publicada describe `sandboxPolicy.readOnly.access` con raíces restringidas. Sin embargo, el schema generado localmente por el paquete oficial npm Codex `0.159.2` sólo declara `type` y `networkAccess` para `ReadOnlySandboxPolicy`; no declara `access`. Enviar un campo desconocido no demuestra que el runtime lo aplique. La adaptación debe comprobar el schema de cada ejecutable y rechazar inferencias cuando falta esta capacidad; no reintentar con lectura global.

Desactivar `features.shell_tool` y `features.unified_exec` no sustituye por sí solo esta garantía: la referencia distingue shell, hooks, agentes, apps y herramientas de imágenes. Los MCP y apps pueden operar fuera del control de red del sandbox. [Referencia de configuración](https://learn.chatgpt.com/docs/config-file/config-reference).

La documentación de MCP permite desactivar servidores identificados mediante `mcp_servers.<id>.enabled = false` y políticas de servidores de plugins. No documenta un comodín que desactive todos los MCP heredados. Tampoco se ha probado que un override de tabla vacía elimine entradas de las capas inferiores. No se considera una alternativa de aislamiento verificada. [Configuración MCP](https://learn.chatgpt.com/docs/extend/mcp).

## Límites de la revisión

No se han ejecutado instaladores descargados, efectuado peticiones de inferencia con claves reales, iniciado sesión ni inspeccionado el almacén de credenciales. No se han verificado builds de macOS ni revisado íntegramente cada dependencia transitiva. Una revisión del código no certifica binarios externos ni garantiza riesgo cero.
