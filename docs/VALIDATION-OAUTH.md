# Validación independiente de Sign in with ChatGPT

Fecha: 2026-10-01. Windows. Este informe cubre la nueva integración OAuth propia de Coucou; las pruebas previas de la integración Codex se documentan en `VALIDATION.md`.

## Alcance y evidencia

La validación utiliza un par RSA sintético generado exclusivamente para fixtures de pruebas, almacenamiento en memoria y sockets HTTP en loopback. Una prueba nativa crea, lee y elimina una entrada aleatoria de prueba del namespace propio de Credential Manager: sólo contiene texto sintético, y comprueba `CRED_PERSIST_LOCAL_MACHINE` con blobs vacíos y de 2.560 bytes. No lee `auth.json`, cookies ni credenciales de otras aplicaciones. Este agente no inició consentimiento real ni solicitudes de inferencia facturables.

El contrato de referencia se encuentra en `RESEARCH-OAUTH.md`: Sign in with ChatGPT oficial, PKCE S256, ID de cliente emitido, verificación OIDC con JWKS, scope `chatgpt.tokens.use.direct` y Responses sin herramientas. No se acepta una clave API como sustitución automática de una sesión fallida.

## Matriz de pruebas

| Frontera | Casos independientes | Evidencia actual |
| --- | --- | --- |
| Almacenamiento protegido | Fallo de una parte o del índice conserva registro anterior; partes ausentes e índice corrupto se rechazan; Unicode respeta límites UTF-16; tamaño máximo; fallo al borrar índice se propaga | PASS: `chatgpt-client/src/adversarial.rs`, más tres pruebas del integrador en `storage.rs` |
| OIDC | Firma válida y alterada; rechazo HS256/unsigned; issuer, audience, nonce, sub, iat, exp y nbf; claims obligatorias; audiencias múltiples y azp; errores no reflejan token | PASS con llave pública sintética: `chatgpt-client/src/oauth_adversarial.rs` |
| Autorización | Listener ya reservado; state/nonce/verifier diferentes; URL vincula challenge S256, resource, callback y cliente dinámico; sin ID token hint | PASS: `chatgpt-client/src/oauth_adversarial.rs` |
| Callback HTTP | GET y Host exactos; duplicados de parámetros y Host; path y fragment confusos; cliente emitido/cambio de cliente; Origin, cuerpo y Transfer-Encoding rechazados; límites; errores redaccionados; callback cancelado; llegada tardía de bytes; goteo lento no extiende deadline | PASS en sockets Windows: `chatgpt-client/src/oauth_adversarial.rs` |
| JSON remoto | OAuth limitado a 256 KiB; catálogo a 4 MiB; catálogo sintético de 300 KiB aceptado; body inválido o demasiado grande no se refleja en errores | PASS con servidor HTTP sintético en loopback; no se modifica ningún endpoint de producción |
| SSE | Terminal completed obligatorio; fallos tras texto parcial; fin prematuro y DONE sin completed; UTF-8/JSON inválidos; CRLF y datos multilínea; límites de línea/evento/stream/texto; errores sanitizados; salida de herramienta/refusal o terminal malformado se rechaza incluso con deltas anteriores; deltas sólo se devuelven tras completed válido sin texto final; texto final autoritativo; MIME ausente permitido, tipos explícitos clasificados y sanitizados | PASS: 11 pruebas en `chatgpt-client/src/responses_adversarial.rs` |
| Estado de sesión | Scopes exactos; proyección pública sin tokens; intento desconocido/repetido/cancelado; logout invalida generación antes de esperar al gate; fallo al persistir logout desactiva memoria; host estable; earliest refresh futuro usa acceso válido o rechaza expirado antes de HTTP; rotación sin verificar no anuncia permiso del plan | PASS: `chatgpt-client/src/adversarial.rs`. La rotación HTTP completa y revocación remota no están simuladas |
| Integración Windows/UI | Estado público sin tokens, selección explícita de backend, eventos de reset y guards contra respuestas antiguas; retry conserva mensaje y adjunto tras error | Inspección de fuente. Build y smoke UI final corresponden al PM y se distinguen abajo |

## Estado de ejecución

Comando independiente ejecutado en `windows`: `cargo test -p chatgpt-client --locked` con la toolchain local del proyecto. Resultado: **44 pruebas pasan; cero fallos**. Son **38 pruebas adversariales independientes** (15 en `adversarial.rs`, 12 en `oauth_adversarial.rs`, 11 en `responses_adversarial.rs`) y seis pruebas del integrador (tres OAuth y tres de almacenamiento). El último resultado completo tardó aproximadamente 0,25 s de ejecución de pruebas, además de la compilación. Los doc-tests, sin casos definidos, también terminaron correctamente.

La primera ejecución encontró dos fallos reales y obtuvo 30/32: un socket aceptado heredaba nonblocking en Windows y un fallo de escritura al desconectar mantenía acceso activo en memoria. Ambos quedaron corregidos y sus pruebas pasan. También se validaron el plazo absoluto del callback, invalidación antes de esperar una operación de credenciales y proyección del estado durante una rotación pendiente.

Después de las cinco regresiones de compatibilidad, el PM ejecutó `cargo test --workspace --locked`: **73 pruebas pasan** (44 OAuth, cuatro del protocolo anterior, 19 de la aplicación y seis del relay). La compilación de los ejemplos incluida en ese comando pasa, pero no ejecuta OAuth ni inferencia del ejemplo `subscription_smoke`.

La prueba real inicial del PM reveló variaciones admitidas por el SDK oficial: catálogo superior a 256 KiB, SSE sin cabecera Content-Type y terminal completed sin texto final. Se añadieron cinco regresiones específicas, todas pasan: límite separado del catálogo, clasificación MIME con errores sin reflejo de cabeceras, acumulación de deltas, terminal estricto y límite acumulado de **1.000.000 bytes**. Nunca se devuelve texto parcial tras EOF, error, status incorrecto, tool/refusal o terminal malformado. Si existe texto final válido, prevalece sobre los deltas. Esta validación con fixtures no sustituye el resultado de la nueva inferencia real del PM.

Se revisó `src-tauri/examples/subscription_smoke.rs`: usa el mismo almacén propio y el navegador oficial; realiza una sola inferencia mínima tras consentimiento y sólo imprime estado, número de modelos, modelo y longitud del texto. No imprime URL de autorización, tokens, identidad, registro ni conversación. Su ejecución real debe coordinarla el PM, con la aplicación detenida para evitar dos escritores del mismo journal.

El PM confirmó que el build final de frontend (`npm run build`) y el release nativo (`cargo build --release -p coucou --features tauri/custom-protocol --locked`) pasan. El ejecutable final arrancó y siguió respondiendo; no apareció un error nuevo de creación de la ventana de ajustes. Los eventos sintéticos SessionStart/SessionEnd llegaron mediante el relay y quedaron registrados. No se modificó hooks.json ni se ejecutó una aprobación real Allow/Deny. Esto es una comprobación de arranque y transporte, no una automatización visual completa. Los resultados corresponden al código y lock presentes en esta ejecución; cambios posteriores requieren la comprobación apropiada.

## Aceptación real coordinada por el PM

El usuario completó el consentimiento en el navegador oficial. Con la aplicación detenida, el ejemplo nativo confirmó el permiso de uso directo del plan, recuperó cinco modelos disponibles y terminó una respuesta con `gpt-5.6-luna`: **`SUBSCRIPTION_INFERENCE_COMPLETED`, dos bytes de texto, proceso finalizado con código 0**. El lector exigió el evento `response.completed` y un estado completed válido. La sesión propia persistida permitió repetir las comprobaciones sin importar ninguna credencial de Codex.

La primera comprobación se detuvo al leer un catálogo mayor que el límite original. Dos peticiones mínimas de inferencia posteriores revelaron las variaciones de MIME y texto terminal descritas arriba; no se presentaron como respuestas correctas. La última petición confirmó la respuesta completa tras corregir esas incompatibilidades. No se enviaron contraseñas o tokens al chat, no se imprimieron cuerpos de respuesta y no se utilizó el backend con clave API.

Esto comprueba consentimiento, catálogo e inferencia oficial para la cuenta y momento probados. No es una inspección de facturación, una prueba de todas las cuentas/modelos ni una automatización de toda la interfaz nativa. Los mensajes y tokens fueron gestionados exclusivamente por el backend de la aplicación dentro del flujo autorizado.

## Límites de aceptación real

Las pruebas de JWT ejercitan el verificador con una llave pública suministrada por el fixture; no prueban la descarga real de JWKS. No se ha simulado la secuencia HTTP completa de dos refrescos, pérdida de respuesta tras consumir un grant, verificación fallida tras rotación o revocación concurrente con la respuesta de renovación. Se inspeccionaron el gate, el checkpoint de rotación y la retirada persistida del grant anterior; esa inspección no equivale a una prueba de extremo a extremo. La suite comprueba la invalidación de memoria/generaciones y el rechazo de resultados incompletos.

Mocks y compilación no demuestran que una cuenta concreta conceda el scope de uso de suscripción, que el catálogo devuelva modelos elegibles ni que una inferencia consuma la suscripción. Esa comprobación exige consentimiento oficial del usuario y una solicitud real explícitamente coordinada. No se puede prometer acceso de todas las cuentas o planes. Ninguna revisión elimina todos los riesgos del sistema operativo, malware ejecutado por el mismo usuario o servicios remotos.
