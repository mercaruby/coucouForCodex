# Investigación OAuth oficial para Coucou en Windows

Fecha: 2026-10-01. Investigación de documentación y código oficial; no se leyeron credenciales de Codex, no se inició OAuth y no se realizó inferencia real.

## Ruta elegida

Implementación Rust propia de **Sign in with ChatGPT (SIWC)**, seguida de llamadas directas a Responses sin herramientas. El registro de clientes locales OSS utiliza `dynamic_agent_client`; OpenAI emite el ID definitivo durante el consentimiento. No requiere clave API, secreto de cliente ni solicitud previa de ID de partner. Apps pagadas/remotas tienen un proceso separado. Un host persiste un UUIDv4 `urn:uuid:…` independiente de la identidad del usuario. [Overview](https://developers.openai.com/siwc/token-sharing-open-source).

Esta ruta elimina la necesidad de conceder al modelo herramientas locales de Codex. La sesión y las credenciales son propias de Coucou; nunca se importa `auth.json` de Codex. Backend API solo por elección explícita del usuario, sin sustitución automática cuando SIWC falla.

## Endpoints verificados

Consulta pública sin autenticación a discovery efectuada durante esta investigación:

| Uso | URL / método |
| --- | --- |
| Discovery | GET `https://auth.openai.com/.well-known/openid-configuration` |
| Issuer | `https://auth.openai.com` |
| Autorizar | GET `https://auth.openai.com/api/accounts/authorize` en navegador del sistema |
| Token / refresh | POST `https://auth.openai.com/api/accounts/oauth/token` |
| JWKS | GET `https://auth.openai.com/.well-known/jwks.json` |
| Revocar | POST `https://auth.openai.com/api/accounts/oauth/revoke` |
| Modelos | GET `https://api.openai.com/v1/models` |
| Inferencia | POST `https://api.openai.com/v1/responses` |
| Gestionar uso | `https://chatgpt.com/settings/usage` |

Discovery anuncia RS256. Validar issuer exacto y que los endpoints descubiertos tienen origen HTTPS `auth.openai.com`; no aceptar endpoints proporcionados por UI, contenido del chat o JWT. Rechazar redirects HTTP de las llamadas con credenciales. Fuente de contraste: [oauth.ts oficial](https://github.com/openai/sign-in-with-chatgpt-devkit/blob/f723814abdccec135b519c451fb6e1992ee5e933/packages/local/src/oauth.ts).

## Contrato de autorización

Preparar listener `http://127.0.0.1:<puerto>/auth/callback` antes de abrir navegador. El puerto puede variar; host, esquema y path deben coincidir exactamente entre autorización e intercambio. Generar state, nonce y verifier aleatorios; PKCE S256 base64url sin padding. Primera autorización: `client_id=dynamic_agent_client`, `agent_name_hint` nombre real de app, `ext_agent_host_id` estable, `response_type=code`, `resource=https://api.openai.com/v1` y scopes `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`.

Callback devuelve code/state/client_id emitido. Reautorización reutiliza el ID guardado y omite agent_name_hint. No guardar ni intercambiar con dynamic_agent_client. POST form del código: grant_type=authorization_code, client_id emitido, code, code_verifier, redirect_uri y resource. Validar firma JWKS del ID token, issuer/audience/expiry/nonce y subject para cuenta existente. Solo habilitar inferencia si scope concedido contiene chatgpt.tokens.use.direct. [Registro oficial](https://developers.openai.com/siwc/token-sharing-open-source/sign-in).

Controles adicionales revisados en código oficial: callback GET/Host/path exactos; parámetros duplicados rechazados; state incorrecto no consume transacción; client_id distinto al guardado rechazado. Verificador RS256 exige iss/aud/exp/iat/sub, comprueba azp cuando existe y para audiencia múltiple. Cada intento tiene timeout/cancelación y límites de conexiones/tamaño. Mantener las credenciales solo en Rust con almacenamiento protegido del sistema; frontend recibe estado público, nunca tokens ni URL con ID token. Conservar el ID emitido aunque caduque el código, para evitar registros duplicados.

## Refresh, perfiles y salida

Serializar refresh por sesión. POST form: grant_type=refresh_token, client_id emitido, refresh_token, resource; omitir scope para conservar grant. Persistir atómicamente reemplazo antes de reutilizarlo. Cada registro mantiene identidad, ID, scopes y tokens separados; email no distingue registros/workspaces. Al cerrar sesión, parar solicitudes e intentar revocar con token=refresh_token, token_type_hint=refresh_token y client_id. HTTP200 vacío confirma revocación. Si falla red y se borra localmente, mostrar que revocación remota no se confirmó. Enlace gestionar uso visible. [Cuentas y sesiones](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions).

Leer expires_in y earliest_refresh_at del resultado. La referencia anuncia access de una hora y refresh de30 días, reemplazado en cada renovación; no asumir duraciones al calcular expiración. Los metadatos internos de access token son opacos. [Referencia de tokens](https://developers.openai.com/siwc/token-sharing-open-source/token-reference).

## Contrato Responses y modelos

GET modelos con el mismo Bearer OAuth de la cuenta seleccionada. El formato SIWC es `models[]` con visibility=list, slug y display_name; conservar orden. Pasar slug como model. POST Responses con Bearer, input array con historial necesario, store=false, stream=true, guidance en instructions o developer. Consumir SSE completo: deltas de texto no prueban éxito; solo response.completed. failed, incomplete y fin sin evento terminal son errores. No usar endpoints privados backend-api. [Modelos e inferencia](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference).

No enviar tools, previous_response_id, max_output_tokens, temperature ni system message explícito en esta integración. Archivos/imágenes son posibles si modelo los acepta; no Files upload API ni audio/video/transcripción. La ruta tiene limitaciones adicionales de parámetros/herramientas; incorporar únicamente los necesarios y documentados. [Limitaciones preview](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations).

## Elegibilidad, errores y comprobación real

El consentimiento puede permitir identidad sin uso del plan. Falta del scope direct mantiene cuenta conectada con uso deshabilitado. user_not_eligible o restricciones403 se muestran; no bucle de OAuth. usage_limit_exceeded pausa y enlaza ajustes, sin inventar reset ni porcentaje. usage_unavailable/503 permite retry acotado conservando credenciales. Refresh revocado/expirado/reutilizado requiere login; fallos temporales no borran sesión. Admission puede devolver detail en vez de error; preservar status/código/requestID sanitizados sin cuerpo sensible. Los fallos SSE pueden aparecer después de texto parcial. [Errores oficiales](https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery).

El [cookbook oficial](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt) pide Plus/Pro para probar. No se puede prometer acceso de todas las cuentas, planes, regiones o modelos sin completar una inferencia con la cuenta concreta. Model catalog tampoco equivale a validación de una solicitud. Las pruebas con mocks no confirman consentimiento ni facturación real.

## Alternativa futura con app-server

La CLI oficial0.159.2 generada no anuncia readOnly.access; campos desconocidos no prueban restricciones. Fuente inspeccionada: openai/codex tag rust-v0.159.2 commit ff6aec96948b70d94983af2641a6b67c94faeff5. ThreadStartParams.environments=[] es experimental y explícitamente deshabilita environment access; shell/apply_patch/view_image requieren has_environment y se eliminan sin entorno. TurnStartParams también preserva vacío; omit/null selecciona default. MCP se registra independientemente y esta opción sola no lo deshabilita. Sería necesaria validación de schema experimental, respuesta y configuración aislada antes de usarlo. [Contrato thread](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/app-server-protocol/src/protocol/v2/thread.rs), [tool registry](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/core/src/tools/spec_plan.rs).

## Procedencia y licencia

Se consultó devkit oficial commit f723814abdccec135b519c451fb6e1992ee5e933. Su SDK tiene licencia Noncommercial especial, distinta de MIT; no se debe copiar o vendorizar silenciosamente dentro de la adaptación MIT. La implementación propia aplica el protocolo publicado y bibliotecas estándar, sin trasplantar código SDK. [Licencia devkit](https://github.com/openai/sign-in-with-chatgpt-devkit/blob/f723814abdccec135b519c451fb6e1992ee5e933/LICENSE). Las restricciones de assets de Coucou siguen documentadas en REVIEW.md.
