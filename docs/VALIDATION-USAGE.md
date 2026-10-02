# Validación independiente del consumo y recuperación de cuota

Fecha: 2026-10-02. Windows. Este informe complementa `VALIDATION-OAUTH.md` y distingue fixtures sintéticas, consulta oficial de sólo lectura e inspección de interfaz.

## Resultado

`cargo test --workspace --locked` pasó con **93 pruebas rutinarias**: 46 de OAuth/Responses, cuatro del protocolo anterior, 37 de la aplicación y seis del relay. Las 37 de aplicación incluyen **18 pruebas independientes de consumo**, una de ellas es el proceso auxiliar inerte del fixture RPC. La aceptación nativa explícita se ejecutó aparte y pasó; permanece ignorada en la suite normal. Después de ajustar su salida para emitir sólo un resumen sin datos personales, se repitió la suite de consumo: **18 pasan, cero fallos, una ignorada**.

El frontend pasó TypeScript (`tsc --noEmit`) y el build de Vite. El fixture `windows/tests/ui-usage.html` no forma parte de las entradas de producción. Utiliza los módulos reales de vistas, estado, consumo y clasificación de errores; sus puentes de inferencia, enlaces y ventana están sustituidos por funciones inertes. No lee archivos, credenciales ni realiza solicitudes de inferencia.

## Pruebas independientes

| Frontera | Casos y resultado |
| --- | --- |
| DTO de consumo | PASS: porcentajes cero reales se preservan; campos ausentes/null siguen desconocidos; métricas camelCase/snake_case; null explícito tiene prioridad; rechazo de valores fuera de rango, fechas en milisegundos y tipos incorrectos. |
| Identidad de límites | PASS: mapa de identificadores autoritativo, incluido mapa vacío; legacy sólo cuando el mapa falta/null; buckets separados; legacy sin identidad no se presenta como Codex; límites de cantidad/longitud y caracteres de control. |
| Redacción | PASS: campos no admitidos y marcadores sintéticos de secreto no salen en DTO ni errores RPC; operaciones de aprobación inesperadas se rechazan; mensajes ajenos se ignoran dentro de un límite. |
| Transporte nativo | PASS: timeout absoluto sin inventar resultado, límite de notificaciones y finalización del proceso auxiliar; scripts, rutas relativas, falsos `.exe` y firmas MZ incompletas se rechazan. El fixture usa el propio ejecutable de pruebas, sin shell, red o credenciales. |
| Cache | PASS: fallo conserva fecha/porcentaje anterior como stale; cambio de ruta borra métricas previas; refresh manual inmediato respeta el intervalo mínimo sin borrar snapshot válido. |
| Cuota en frontend | PASS: error genérico que menciona límites sigue siendo genérico; 429 temporal no equivale a bloqueo definitivo; cuerpo de error desconocido no aparece en el aviso; enlace de gestión fijado a `https://chatgpt.com/settings/usage`. |

Los casos Rust están en `windows/src-tauri/src/usage_adversarial.rs`. La prueba ignorada admite el ejecutable instalado oficialmente o una ruta elegida explícitamente; sólo imprime fuente, presencia/número de ventanas y validez de porcentajes/unidades, nunca porcentajes reales, plan, fechas de reinicio, identidad o tokens.

## Aceptación nativa de sólo lectura

Con autorización del usuario, se comprobó el ejecutable nativo oficial instalado y se ejecutó la consulta de consumo mediante el app-server oficial. Resultado: **PASS en aproximadamente 0,76 s**, `source=codex-app-server`, `metricsPresent=true`, dos ventanas, porcentajes y unidades válidos. El proceso se ejecutó oculto. Sólo se utilizaron inicialización, notificación de inicializado y `account/rateLimits/read`; **cero inferencias** y ningún `account/read`, inicio de sesión o importación de credenciales.

Este informe y las capturas públicas omiten la cuota privada observada, el plan, las épocas de reinicio y la ruta personal del binario. La comprobación real se realizó con ruta explícita verificada; el descubrimiento automático del cliente instalado se revisó en fuente, sin repetir innecesariamente la consulta real tras ese cambio.

## Interfaz real con datos sintéticos

Se abrió el fixture local en Edge con los componentes de producción. Los seis checks iniciales pasan. Las capturas sólo contienen valores sintéticos: 0 %, 87 % y métricas desconocidas.

- A **640 × 310 px**, la card mide 618 px de ancho sin desbordamiento horizontal. Las dos ventanas conocidas exponen `role=progressbar`, valores 0/87 y rango 0–100; las desconocidas muestran «Sin dato» y no exponen un porcentaje inventado. La zona interna permite desplazamiento por teclado para llegar al estado de ChatGPT.
- A **320 × 310 px**, se detectó inicialmente texto recortado entre las dos ventanas. Tras el ajuste del PM, el container query usa una columna: zona de scroll de 194/194 px y ventanas de 191/191 px, sin desbordamiento horizontal. El scroll vertical conserva acceso al contenido. Esto valida los componentes estrechos; la ventana nativa mantiene su ancho previsto.
- El caso sin datos muestra su estado explícito y configuración de Codex, sin representar ausencia como cero. El caso stale muestra «Lectura anterior» y conserva sus porcentajes con una advertencia visible.
- Se envió un mensaje sintético con un adjunto virtual `fixture.txt` a un puente que rechaza con cuota. El aviso contiene texto fijo, sin el marcador privado del error. «Gestionar uso» registra exclusivamente el enlace oficial; «Volver al mensaje» restaura exactamente el borrador, conserva el adjunto y deja cero mensajes fallidos en el historial.
- A **640 × 220 px**, las tres acciones del aviso caben. A **320 × 220 px**, el stack se desplaza verticalmente; Tab alcanza «Consumo» y lleva el scroll hasta su máximo, dejando accesibles todas las acciones. Se verificó el atributo `inert` en todas las vistas ocultas del fixture.

El PM repitió por separado el regreso al borrador en su navegador integrado. Las capturas locales de evidencia son `usage-panel-640.png`, `usage-panel-640-bottom.png`, `usage-panel-320.png`, `usage-note-640.png` y `usage-note-320.png`; no se incluyen datos reales de cuenta.

## Comprobación final del PM

El build final `cargo build --release -p coucou --features tauri/custom-protocol --locked` pasó, después del build final de TypeScript/Vite. El PM arrancó el ejecutable propio reconstruido y confirmó que sigue respondiendo. Los eventos sintéticos SessionStart/SessionEnd llegaron por el relay una vez listo el servidor; no se registró un error nuevo en el arranque o esa entrega. No se modificó la configuración de hooks ni se ejecutaron aprobaciones reales. Esta es una comprobación nativa de arranque/transporte, distinta de la validación visual de componentes en navegador; no afirma que se automatizara toda la UI nativa ni que se eliminara un límite remoto de ChatGPT.

## Límites de la evidencia

No se agotó una cuota real para provocar bloqueo ni se realizó inferencia adicional durante esta fase. Las pruebas RPC sintéticas ejercitan el parser y el lector acotado; no simulan todos los cambios de cuenta de un servicio remoto. Los guards contra respuestas de sesiones anteriores y cambios concurrentes se inspeccionaron en fuente, sin automatizar todas las carreras de la interfaz nativa.

La consulta del cliente oficial informa el consumo de Codex que éste publica. No demuestra que el presupuesto de una integración OAuth propia de Coucou sea el mismo; la interfaz conserva ambas fuentes separadas y presenta el porcentaje de ChatGPT en Coucou como no publicado. La aceptación nativa del PM se documenta arriba como comprobación distinta del fixture de navegador.
