// Bulk seeding for the read-heavy scenario. Called from setup().
//
// Idempotent: signups that hit 409 (user already exists) are treated
// as success — todos are NOT re-created in that case, so reruns are
// fast (~ms per existing user, no DB writes).

import http from 'k6/http';
import { BASE_URL } from './config.js';
import { signup } from './auth.js';
import { params, Endpoint } from './checks.js';

// Seeds N users, each with `todosPerUser` todos. Returns an array of
// `{ email, password }` records for the VU function to log in with.
export function seedUsersWithTodos(n, todosPerUser) {
    const users = [];
    const password = 'k6-load-test-password';

    for (let i = 1; i <= n; i++) {
        const email = `loadtest-u${i}@example.test`;
        const res = signup(email, password);
        const isNewUser = res.status === 303;

        if (isNewUser) {
            // Signup auto-logs in via Set-Cookie on the 303; the cookie jar
            // captures it and subsequent posts here are authenticated.
            for (let j = 1; j <= todosPerUser; j++) {
                const todoRes = http.post(
                    `${BASE_URL}/todos`,
                    { title: `task-${i}-${j}` },
                    params(Endpoint.TodoCreate, { redirects: 0 }),
                );
                // POST /todos returns 201 on htmx-style success.
                if (todoRes.status !== 201 && todoRes.status !== 200) {
                    throw new Error(
                        `seed todo ${i}/${j} unexpected status ${todoRes.status}`,
                    );
                }
            }
        }
        users.push({ email, password });
    }

    return users;
}
