async function build() {
    const body = document.createElement("main");
    document.body.appendChild(body);
    const features = await (await fetch("/api/features")).json();
    const spotify_token = (await (await fetch("/api/spotify_token")).text()).trim();
    for (const feature_name of features) {
        const create_button = document.createElement("button");
        create_button.innerText = feature_name;
        create_button.addEventListener("click", async () => {
            while (body.firstChild !== null) {
                body.removeChild(body.firstChild);
            }
            const header = document.createElement("p");
            header.innerText = feature_name;
            body.appendChild(header);
            const track = document.createElement("p");
            async function reloadRandom() {
                const details = (await (await fetch(`/api/features/${feature_name}/tracks/random_untrained`)).json());
                const artists = details.artists.map(artist => artist.name).join(", ");
                track.innerText = `${artists} – ${details.name}`;
                body.dataset.id = details.id;
                const uri = `spotify:track:${details.id}`;
                fetch("https://api.spotify.com/v1/me/player/play", { method: "PUT", headers: { "Authorization": `Bearer ${spotify_token}`, "Content-Type": "application/json" }, body: JSON.stringify({ "uris": [uri] }) });
            }
            async function rateAndReload(rating) {
                await fetch(`/api/features/${feature_name}/tracks/${body.dataset.id}/rate/${rating}`, { method: "POST" });
                await reloadRandom();
            }
            const downvote = document.createElement("button");
            downvote.innerText = "0";
            downvote.addEventListener("click", () => rateAndReload(0));
            const upvote = document.createElement("button");
            upvote.innerText = "1";
            upvote.addEventListener("click", () => rateAndReload(1));
            body.appendChild(downvote);
            body.appendChild(upvote);
            body.appendChild(document.createElement("br"));
            body.appendChild(track);
            await reloadRandom();
        });
        body.appendChild(create_button);
    }
}
build();